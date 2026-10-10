//! GIFs: the composer's picker (a KLIPY search in a popover, ported from
//! Bubo) and playback for a GIF on a `gtk::Picture`, which on its own
//! shows only the first frame.

use crate::i18n::{gettext, gif_failure_message};
use crate::rt;
use crate::settings as keys;
use crate::window::SkaliWindow;
use adw::prelude::*;
use gtk::gdk_pixbuf::{PixbufAnimation, PixbufAnimationIter, PixbufLoader};
use gtk::{gdk, glib};
use skali_core::gif::{self, Gif};
use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

/// A search runs once typing has paused this long.
const DEBOUNCE: Duration = Duration::from_millis(350);
/// A tile in the picker's grid, square.
const TILE: i32 = 112;

/// The "GIF" button for a composer: it opens the picker, and `on_pick`
/// gets the button and the GIF chosen. With the search empty the picker
/// shows the GIFs sent most recently.
pub fn picker_button(
    window: &SkaliWindow,
    on_pick: impl Fn(&gtk::MenuButton, Gif) + 'static,
) -> gtk::MenuButton {
    let search = gtk::SearchEntry::builder()
        .placeholder_text(gettext("Search GIFs"))
        .build();
    let status = gtk::Label::builder()
        .css_classes(["dim-label"])
        .wrap(true)
        .justify(gtk::Justification::Center)
        .margin_top(12)
        .margin_bottom(12)
        .build();
    let grid = gtk::FlowBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .min_children_per_line(3)
        .max_children_per_line(3)
        .homogeneous(true)
        .row_spacing(4)
        .column_spacing(4)
        .valign(gtk::Align::Start)
        .build();
    let scroller = gtk::ScrolledWindow::builder()
        .child(&grid)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .height_request(400)
        .build();
    let column = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(6)
        .width_request(372)
        .build();
    column.append(&search);
    column.append(&status);
    column.append(&scroller);
    let popover = gtk::Popover::builder().child(&column).build();
    let button = gtk::MenuButton::builder()
        // A child, not a label: a label brings a dropdown arrow.
        .child(&gtk::Label::new(Some("GIF")))
        .tooltip_text(gettext("Send a GIF"))
        .popover(&popover)
        .css_classes(["circular", "gif-button"])
        .build();

    let on_pick: Rc<dyn Fn(Gif)> = Rc::new(glib::clone!(
        #[weak]
        button,
        move |gif| on_pick(&button, gif)
    ));
    let fill = {
        let (grid, popover, on_pick) = (grid.clone(), popover.clone(), on_pick.clone());
        move |gifs: Vec<Gif>| {
            while let Some(child) = grid.first_child() {
                grid.remove(&child);
            }
            for gif in gifs {
                grid.append(&tile(gif, &popover, &on_pick));
            }
        }
    };
    let show_recent = {
        let (window, status, fill) = (window.downgrade(), status.clone(), fill.clone());
        move || {
            let Some(window) = window.upgrade() else {
                return;
            };
            let recent = gif::history(&window.db().borrow());
            status.set_label(&if recent.is_empty() {
                gettext("Type to search KLIPY")
            } else {
                gettext("Recently sent")
            });
            status.set_visible(true);
            fill(recent);
        }
    };
    popover.connect_show(glib::clone!(
        #[strong]
        show_recent,
        #[weak]
        search,
        move |_| {
            search.grab_focus();
            if search.text().trim().is_empty() {
                show_recent();
            }
        }
    ));

    // Each keystroke bumps the generation; a search runs, and its answer
    // lands, only while it is still the newest.
    let generation = Rc::new(Cell::new(0u64));
    let window = window.downgrade();
    search.connect_search_changed(move |search| {
        let query = search.text().to_string();
        generation.set(generation.get() + 1);
        let mine = generation.get();
        if query.trim().is_empty() {
            show_recent();
            return;
        }
        glib::timeout_add_local_once(
            DEBOUNCE,
            glib::clone!(
                #[strong]
                generation,
                #[strong]
                status,
                #[strong]
                fill,
                #[strong]
                window,
                move || {
                    if generation.get() != mine {
                        return;
                    }
                    let Some(window) = window.upgrade() else {
                        return;
                    };
                    let key = window.settings().string(keys::KLIPY_API_KEY).to_string();
                    status.set_label(&gettext("Searching…"));
                    status.set_visible(true);
                    rt::run(
                        async move { gif::search(&key, &query, 0).await },
                        move |result| {
                            if generation.get() != mine {
                                return;
                            }
                            match result {
                                Ok(gifs) if gifs.is_empty() => {
                                    status.set_label(&gettext("No GIFs found"));
                                    fill(gifs);
                                }
                                Ok(gifs) => {
                                    status.set_visible(false);
                                    fill(gifs);
                                }
                                Err(error) => {
                                    tracing::warn!("GIF search failed: {error}");
                                    status.set_label(&gif_failure_message(&error));
                                    fill(Vec::new());
                                }
                            }
                        },
                    );
                }
            ),
        );
    });
    button
}

/// One result: its still while the animated preview downloads, then the
/// preview, playing. Clicking it closes the picker and picks it.
fn tile(gif: Gif, popover: &gtk::Popover, on_pick: &Rc<dyn Fn(Gif)>) -> gtk::Widget {
    let picture = gtk::Picture::builder()
        .content_fit(gtk::ContentFit::Cover)
        .can_shrink(true)
        .overflow(gtk::Overflow::Hidden)
        .width_request(TILE)
        .height_request(TILE)
        .build();
    let button = gtk::Button::builder()
        .child(&picture)
        .css_classes(["flat", "gif-tile"])
        .build();
    // The preview may beat the still; then the still is dropped.
    let playing = Rc::new(Cell::new(false));
    if !gif.still.is_empty() {
        let url = gif.still.clone();
        rt::run(
            async move { gif::thumbnail(&url).await },
            glib::clone!(
                #[weak]
                picture,
                #[strong]
                playing,
                move |result| {
                    if let Ok(bytes) = result
                        && !playing.get()
                    {
                        let _ = show_image(&picture, &bytes);
                    }
                }
            ),
        );
    }
    let url = gif.preview.clone();
    rt::run(
        async move { gif::thumbnail(&url).await },
        glib::clone!(
            #[weak]
            picture,
            move |result| match result {
                Ok(bytes) => {
                    if show_image(&picture, &bytes).is_some() {
                        playing.set(true);
                    }
                }
                Err(error) => tracing::debug!("a GIF preview didn't load: {error}"),
            }
        ),
    );
    button.connect_clicked(glib::clone!(
        #[weak]
        popover,
        #[strong]
        on_pick,
        move |_| {
            popover.popdown();
            on_pick(gif.clone());
        }
    ));
    button.upcast()
}

/// Show an image on `picture`, its pixel size back, or None if it isn't
/// one. A GIF plays for as long as the picture lives, its frames
/// advancing only while it's on screen.
pub fn show_image(picture: &gtk::Picture, bytes: &[u8]) -> Option<(i32, i32)> {
    if !bytes.starts_with(b"GIF8") {
        let texture = gdk::Texture::from_bytes(&glib::Bytes::from(bytes)).ok()?;
        picture.set_paintable(Some(&texture));
        return Some((texture.width(), texture.height()));
    }
    let loader = PixbufLoader::new();
    loader.write(bytes).ok()?;
    loader.close().ok()?;
    let animation: PixbufAnimation = loader.animation()?;
    let iter = animation.iter(None);
    picture.set_paintable(Some(&gdk::Texture::for_pixbuf(&iter.pixbuf())));
    if !animation.is_static_image() {
        next_frame(picture.downgrade(), iter);
    }
    Some((animation.width(), animation.height()))
}

fn next_frame(picture: glib::WeakRef<gtk::Picture>, iter: PixbufAnimationIter) {
    let delay = iter
        .delay_time()
        .unwrap_or(Duration::from_millis(100))
        .max(Duration::from_millis(20));
    glib::timeout_add_local_once(delay, move || {
        let Some(shown) = picture.upgrade() else {
            return;
        };
        if shown.is_mapped() {
            iter.advance(std::time::SystemTime::now());
            shown.set_paintable(Some(&gdk::Texture::for_pixbuf(&iter.pixbuf())));
        }
        next_frame(picture, iter);
    });
}
