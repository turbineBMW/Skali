//! An image at the size of the window: a click on a chat image, or on an
//! image in a document, opens it here. One bigger than the window fits
//! it, and a click shows it at full size, scrolling.

use adw::prelude::*;

/// The header bar's height, which the dialog's content size includes.
const HEADER_HEIGHT: i32 = 47;
/// The smallest the dialog gets, for a tiny image.
const MIN_WIDTH: i32 = 360;
const MIN_HEIGHT: i32 = 240;

/// Show `bytes` (any image GTK reads; a GIF plays) over `parent`'s window.
pub fn show(parent: &impl IsA<gtk::Widget>, title: &str, bytes: &[u8]) {
    let picture = gtk::Picture::builder()
        .content_fit(gtk::ContentFit::Contain)
        .can_shrink(true)
        .hexpand(true)
        .vexpand(true)
        .alternative_text(title)
        .build();
    let Some((width, height)) = super::gifs::show_image(&picture, bytes) else {
        tracing::warn!("the image {title} didn't decode");
        return;
    };
    let (width, height) = (width.max(1), height.max(1));
    let scroller = gtk::ScrolledWindow::builder().child(&picture).build();
    let header = adw::HeaderBar::new();
    header.set_title_widget(Some(&adw::WindowTitle::new(
        title,
        &format!("{width} × {height}"),
    )));
    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&header);
    toolbar.set_content(Some(&scroller));

    // The image in most of the window, never past its own pixels.
    let (room_width, room_height) = parent
        .root()
        .map(|root| {
            (
                root.width() * 9 / 10,
                root.height() * 9 / 10 - HEADER_HEIGHT,
            )
        })
        .unwrap_or((1000, 700));
    let scale = (f64::from(room_width) / f64::from(width))
        .min(f64::from(room_height) / f64::from(height))
        .min(1.0);
    let dialog = adw::Dialog::builder()
        .title(title)
        .content_width(((f64::from(width) * scale).round() as i32).max(MIN_WIDTH))
        .content_height(
            ((f64::from(height) * scale).round() as i32).max(MIN_HEIGHT) + HEADER_HEIGHT,
        )
        .child(&toolbar)
        .build();

    // Shrunk to fit: a click toggles full size, which scrolls.
    if scale < 1.0 {
        picture.set_cursor_from_name(Some("zoom-in"));
        let click = gtk::GestureClick::new();
        click.connect_released(move |gesture, _, _, _| {
            let Some(picture) = gesture.widget().and_downcast::<gtk::Picture>() else {
                return;
            };
            let fitted = !picture.can_shrink();
            picture.set_can_shrink(fitted);
            picture.set_cursor_from_name(Some(if fitted { "zoom-in" } else { "zoom-out" }));
        });
        picture.add_controller(click);
    }
    dialog.present(Some(parent));
}
