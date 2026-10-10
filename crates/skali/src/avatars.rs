//! People's pictures. Fetched once per URL, kept as a texture for the
//! session and as a file in the cache dir across sessions. Every avatar
//! widget waiting on the same URL is filled when it arrives.

use crate::rt;
use adw::prelude::*;
use gtk::gdk;
use gtk::glib;
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Default)]
struct Cache {
    textures: HashMap<String, gdk::Texture>,
    /// Avatars waiting on a fetch in flight, by URL.
    waiting: HashMap<String, Vec<glib::WeakRef<adw::Avatar>>>,
}

thread_local! {
    static CACHE: RefCell<Cache> = RefCell::new(Cache::default());
}

fn cache_dir() -> PathBuf {
    glib::user_cache_dir().join("skali/avatars")
}

fn cache_path(url: &str) -> PathBuf {
    let name = glib::compute_checksum_for_string(glib::ChecksumType::Sha256, url)
        .map(|sum| sum.to_string())
        .unwrap_or_else(|| url.replace('/', "_"));
    cache_dir().join(name)
}

/// The picture's bytes if they are on disk already (a document that
/// inlines avatars reads them here; a miss means initials this time).
pub fn cached_bytes(url: &str) -> Option<Vec<u8>> {
    if url.is_empty() {
        return None;
    }
    std::fs::read(cache_path(url)).ok()
}

/// Show the picture at `url` in `avatar`, now if we have it, when it
/// arrives if not. An empty URL leaves the initials.
pub fn load_into(avatar: &adw::Avatar, url: &str) {
    if url.is_empty() {
        return;
    }
    let pending = CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        if let Some(texture) = cache.textures.get(url) {
            avatar.set_custom_image(Some(texture));
            return false;
        }
        let waiting = cache.waiting.entry(url.to_string()).or_default();
        waiting.push(avatar.downgrade());
        waiting.len() == 1
    });
    if !pending {
        return;
    }
    let url_owned = url.to_string();
    let path = cache_path(url);
    rt::run(
        async move {
            if let Ok(bytes) = tokio::fs::read(&path).await {
                return Ok(bytes);
            }
            let bytes = skali_core::api::fetch_public_bytes(&url_owned).await?;
            let _ = tokio::fs::create_dir_all(cache_dir()).await;
            let _ = tokio::fs::write(&path, &bytes).await;
            Ok::<_, skali_core::error::ApiError>(bytes)
        },
        {
            let url = url.to_string();
            move |result| {
                let texture = result.map_err(|error| error.to_string()).and_then(|bytes| {
                    gdk::Texture::from_bytes(&glib::Bytes::from_owned(bytes))
                        .map_err(|error| error.to_string())
                });
                let waiting = CACHE.with(|cache| {
                    let mut cache = cache.borrow_mut();
                    if let Ok(texture) = &texture {
                        cache.textures.insert(url.clone(), texture.clone());
                    }
                    cache.waiting.remove(&url).unwrap_or_default()
                });
                match texture {
                    Ok(texture) => {
                        for avatar in waiting.iter().filter_map(glib::WeakRef::upgrade) {
                            avatar.set_custom_image(Some(&texture));
                        }
                    }
                    Err(error) => tracing::debug!("no avatar from {url}: {error}"),
                }
            }
        },
    );
}
