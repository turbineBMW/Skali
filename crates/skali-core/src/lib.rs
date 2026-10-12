//! Everything Skali knows about Basecamp that has nothing to do with
//! widgets: the HTTP client, OAuth through Launchpad, the keyring, the
//! SQLite cache and the models. All of it is unit-tested here; the GTK
//! crate only wires it to the screen.

pub mod api;
pub mod auth;
pub mod darkmode;
pub mod db;
pub mod embed;
pub mod error;
pub mod feed;
pub mod gif;
pub mod links;
pub mod models;
pub mod omarchy;
pub mod richtext;
pub mod schedule;
pub mod secrets;
pub mod summary;

/// What Basecamp sees in the `User-Agent` header, which it requires: the
/// app's name and where to find it.
pub const USER_AGENT: &str = concat!(
    "Skali/",
    env!("CARGO_PKG_VERSION"),
    " (https://github.com/turbinebmw/Skali)"
);

/// A client for the open web (KLIPY, link previews), not Basecamp: no
/// sign-in, the same User-Agent.
pub(crate) fn web() -> &'static reqwest::Client {
    static CLIENT: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .timeout(std::time::Duration::from_secs(20))
            .build()
            .expect("a client with no custom TLS builds")
    })
}
