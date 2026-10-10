//! Skali: a GTK 4 / libadwaita Basecamp client.

mod accent;
mod application;
mod avatars;
mod config;
mod dialogs;
mod i18n;
mod omarchy;
mod pages;
mod rt;
mod settings;
mod widgets;
mod window;

use gtk::prelude::*;
use gtk::{gio, glib};
use std::future::Future;

fn main() -> glib::ExitCode {
    // Before the log opens a new cache directory.
    let moved: Vec<_> = [glib::user_data_dir(), glib::user_cache_dir()]
        .iter()
        .map(|base| (base.clone(), move_old_dir(base)))
        .collect();
    init_logging();
    for (base, result) in moved {
        match result {
            Ok(true) => tracing::info!("moved {}/campfe to skali", base.display()),
            Ok(false) => {}
            Err(error) => tracing::warn!("could not move {}/campfe: {error}", base.display()),
        }
    }
    i18n::init();

    // `skali api [METHOD] <path> [json body]`: one raw API call with the
    // stored sign-in, the answer printed as it came. For looking at what
    // Basecamp really sends, and for seeding a sandbox account. Writes
    // obey SKALI_WRITE_ACCOUNTS like the app does.
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("upload") {
        let Some(path) = args.get(2) else {
            eprintln!("usage: skali upload <file>");
            return glib::ExitCode::FAILURE;
        };
        return upload(std::path::Path::new(path));
    }
    if matches!(args.get(1).map(String::as_str), Some("api" | "dump")) {
        let rest = &args[2..];
        let (method, rest) = match rest.first().map(String::as_str) {
            Some(m @ ("GET" | "POST" | "PUT" | "DELETE")) => (m, &rest[1..]),
            _ => ("GET", rest),
        };
        let path = rest.first().map(String::as_str).unwrap_or("/projects.json");
        return api_call(method, path, rest.get(1).map(String::as_str));
    }

    gio::resources_register_include!("skali.gresource")
        .expect("the resource bundle is compiled in");
    glib::set_application_name("Skali");

    let app = application::SkaliApplication::new();
    app.run()
}

/// Skali was Campfe at first. Its directory under `base` (the data or the
/// cache directory) becomes Skali's, files named for it included
/// (`campfe.db` and its journal, `campfe.log`), unless Skali has one
/// already. `true` if something moved.
fn move_old_dir(base: &std::path::Path) -> std::io::Result<bool> {
    let (old, new) = (base.join("campfe"), base.join("skali"));
    if !old.is_dir() || new.exists() {
        return Ok(false);
    }
    std::fs::rename(&old, &new)?;
    for entry in std::fs::read_dir(&new)?.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if let Some(rest) = name.strip_prefix("campfe.") {
            std::fs::rename(entry.path(), new.join(format!("skali.{rest}")))?;
        }
    }
    Ok(true)
}

/// Two logs: stderr at whatever `SKALI_LOG` asks (warnings by default),
/// and `~/.cache/skali/skali.log` at debug, always, so a problem can be
/// looked into after the fact.
///
/// Never log a token: `TokenSet`'s Debug hides them, but nothing stops a
/// `{:?}` of a request that carries one.
fn init_logging() {
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;
    use tracing_subscriber::{EnvFilter, Layer};

    let stderr = tracing_subscriber::fmt::layer()
        .with_writer(std::io::stderr)
        .with_filter(
            EnvFilter::try_from_env("SKALI_LOG").unwrap_or_else(|_| EnvFilter::new("warn")),
        );
    let dir = glib::user_cache_dir().join("skali");
    let _ = std::fs::create_dir_all(&dir);
    let file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join("skali.log"))
        .ok();
    let file_layer = file.map(|file| {
        tracing_subscriber::fmt::layer()
            .with_ansi(false)
            .with_writer(std::sync::Mutex::new(file))
            .with_filter(EnvFilter::new("skali=debug,skali_core=debug"))
    });
    tracing_subscriber::registry()
        .with(stderr)
        .with(file_layer)
        .init();
    tracing::info!("skali {} starting", config::VERSION);
}

/// `skali upload <file>`: an attachment for rich text; prints the reply
/// (its `attachable_sgid` is what a `<bc-attachment>` takes).
fn upload(path: &std::path::Path) -> glib::ExitCode {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) => {
            eprintln!("could not read {}: {error}", path.display());
            return glib::ExitCode::FAILURE;
        }
    };
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "file".into());
    let content_type = gio::content_type_guess(Some(path), Some(bytes.as_slice()))
        .0
        .to_string();
    let content_type = gio::content_type_get_mime_type(&content_type)
        .map(|m| m.to_string())
        .unwrap_or_else(|| "application/octet-stream".into());
    with_first_client(|client| async move {
        client
            .upload_attachment(&name, &content_type, bytes)
            .await
            .map(|reply| reply.to_string())
    })
}

/// The developer's window into the API: the first account's client, one
/// call, the body as it came. `/path.json` is account-relative; a full URL
/// (from a `url` field) is used as is.
fn api_call(method: &str, path: &str, body: Option<&str>) -> glib::ExitCode {
    let body: Option<serde_json::Value> = match body.map(serde_json::from_str) {
        Some(Ok(body)) => Some(body),
        Some(Err(error)) => {
            eprintln!("the body isn't JSON: {error}");
            return glib::ExitCode::FAILURE;
        }
        None => None,
    };
    let method = method.to_string();
    let path = path.to_string();
    with_first_client(|client| async move { client.call_raw(&method, &path, body.as_ref()).await })
}

/// Run one call against the first account's client with the stored
/// sign-in, printing what it returns.
fn with_first_client<F, Fut>(call: F) -> glib::ExitCode
where
    F: FnOnce(skali_core::api::Client) -> Fut,
    Fut: Future<Output = skali_core::error::Result<String>>,
{
    use skali_core::api::Client;
    use skali_core::db::Database;
    use std::sync::Arc;

    let db_path = glib::user_data_dir().join("skali/skali.db");
    let accounts = Database::open(&db_path)
        .and_then(|db| db.accounts())
        .unwrap_or_else(|error| {
            eprintln!("could not read the accounts: {error}");
            Vec::new()
        });
    // SKALI_API_ACCOUNT=<id> picks the account; otherwise the first one.
    let wanted = std::env::var("SKALI_API_ACCOUNT")
        .ok()
        .and_then(|id| id.parse::<i64>().ok());
    let account = match wanted {
        Some(id) => accounts.iter().find(|a| a.id == id).cloned(),
        None => accounts.first().cloned(),
    };
    let Some(account) = account else {
        eprintln!("no such account: sign in first (or check SKALI_API_ACCOUNT)");
        return glib::ExitCode::FAILURE;
    };
    rt::handle().block_on(async move {
        let tokens = match skali_core::secrets::load_tokens(account.identity_id).await {
            Ok(Some(tokens)) => tokens,
            Ok(None) => {
                eprintln!("no tokens in the keyring: sign in first");
                return glib::ExitCode::FAILURE;
            }
            Err(error) => {
                eprintln!("keyring: {error}");
                return glib::ExitCode::FAILURE;
            }
        };
        let token = Arc::new(tokio::sync::RwLock::new(tokens.access_token));
        let client = Client::new(&account.href, token).expect("a client");
        match call(client).await {
            Ok(body) => {
                println!("{body}");
                glib::ExitCode::SUCCESS
            }
            Err(error) => {
                eprintln!("{error}");
                glib::ExitCode::FAILURE
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::move_old_dir;

    #[test]
    fn the_old_directory_moves_once() {
        let base = tempfile::tempdir().unwrap();
        let old = base.path().join("campfe");
        std::fs::create_dir(&old).unwrap();
        for name in ["campfe.db", "campfe.db-wal", "mediakeys"] {
            std::fs::write(old.join(name), name).unwrap();
        }
        assert!(move_old_dir(base.path()).unwrap());
        let new = base.path().join("skali");
        assert!(!old.exists());
        assert_eq!(std::fs::read(new.join("skali.db")).unwrap(), b"campfe.db");
        assert_eq!(
            std::fs::read(new.join("skali.db-wal")).unwrap(),
            b"campfe.db-wal"
        );
        assert!(new.join("mediakeys").exists());

        // Skali's own directory is never overwritten.
        std::fs::create_dir(&old).unwrap();
        assert!(!move_old_dir(base.path()).unwrap());
        assert!(old.exists());
        assert!(!move_old_dir(&base.path().join("nothing-here")).unwrap());
    }
}
