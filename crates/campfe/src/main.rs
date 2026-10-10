//! Campfe: a GTK 4 / libadwaita Basecamp client.

mod accent;
mod application;
mod config;
mod i18n;
mod omarchy;
mod pages;
mod rt;
mod settings;
mod window;

use gtk::prelude::*;
use gtk::{gio, glib};

fn main() -> glib::ExitCode {
    init_logging();
    i18n::init();

    // `campfe api [METHOD] <path> [json body]`: one raw API call with the
    // stored sign-in, the answer printed as it came. For looking at what
    // Basecamp really sends, and for seeding a sandbox account. Writes
    // obey CAMPFE_WRITE_ACCOUNTS like the app does.
    let args: Vec<String> = std::env::args().collect();
    if matches!(args.get(1).map(String::as_str), Some("api" | "dump")) {
        let rest = &args[2..];
        let (method, rest) = match rest.first().map(String::as_str) {
            Some(m @ ("GET" | "POST" | "PUT" | "DELETE")) => (m, &rest[1..]),
            _ => ("GET", rest),
        };
        let path = rest.first().map(String::as_str).unwrap_or("/projects.json");
        return api_call(method, path, rest.get(1).map(String::as_str));
    }

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

/// The developer's window into the API: the first account's client, one
/// call, the body as it came. `/path.json` is account-relative; a full URL
/// (from a `url` field) is used as is.
fn api_call(method: &str, path: &str, body: Option<&str>) -> glib::ExitCode {
    use campfe_core::api::Client;
    use campfe_core::db::Database;
    use std::sync::Arc;

    let db_path = glib::user_data_dir().join("campfe/campfe.db");
    let accounts = Database::open(&db_path)
        .and_then(|db| db.accounts())
        .unwrap_or_else(|error| {
            eprintln!("could not read the accounts: {error}");
            Vec::new()
        });
    let Some(account) = accounts.first() else {
        eprintln!("no account: sign in first");
        return glib::ExitCode::FAILURE;
    };
    let account = account.clone();
    let body = body.map(str::to_string);
    rt::handle().block_on(async move {
        let body = body.as_deref();
        let tokens = match campfe_core::secrets::load_tokens(account.identity_id).await {
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
        let body: Option<serde_json::Value> = match body.map(serde_json::from_str) {
            Some(Ok(body)) => Some(body),
            Some(Err(error)) => {
                eprintln!("the body isn't JSON: {error}");
                return glib::ExitCode::FAILURE;
            }
            None => None,
        };
        match client.call_raw(method, path, body.as_ref()).await {
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
