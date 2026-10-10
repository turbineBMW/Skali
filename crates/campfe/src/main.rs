//! Campfe: a GTK 4 / libadwaita Basecamp client.

mod accent;
mod application;
mod config;
mod i18n;
mod omarchy;
mod rt;
mod settings;
mod window;

use gtk::prelude::*;
use gtk::{gio, glib};

fn main() -> glib::ExitCode {
    init_logging();
    i18n::init();

    gio::resources_register_include!("campfe.gresource")
        .expect("the resource bundle is compiled in");
    glib::set_application_name("Campfe");

    let app = application::CampfeApplication::new();
    app.run()
}

/// Two logs: stderr at whatever `CAMPFE_LOG` asks (warnings by default),
/// and `~/.cache/campfe/campfe.log` at debug, always, so a problem can be
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
            EnvFilter::try_from_env("CAMPFE_LOG").unwrap_or_else(|_| EnvFilter::new("warn")),
        );
    let dir = glib::user_cache_dir().join("campfe");
    let _ = std::fs::create_dir_all(&dir);
    let file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join("campfe.log"))
        .ok();
    let file_layer = file.map(|file| {
        tracing_subscriber::fmt::layer()
            .with_ansi(false)
            .with_writer(std::sync::Mutex::new(file))
            .with_filter(EnvFilter::new("campfe=debug,campfe_core=debug"))
    });
    tracing_subscriber::registry()
        .with(stderr)
        .with(file_layer)
        .init();
    tracing::info!("campfe {} starting", config::VERSION);
}
