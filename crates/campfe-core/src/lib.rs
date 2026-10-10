//! Everything Campfe knows about Basecamp that has nothing to do with
//! widgets: the HTTP client, OAuth through Launchpad, the keyring, the
//! SQLite cache and the models. All of it is unit-tested here; the GTK
//! crate only wires it to the screen.

pub mod api;
pub mod auth;
pub mod darkmode;
pub mod db;
pub mod error;
pub mod models;
pub mod omarchy;
pub mod richtext;
pub mod secrets;

/// What Basecamp sees in the `User-Agent` header, which it requires: the
/// app's name and where to find it.
pub const USER_AGENT: &str = concat!(
    "Campfe/",
    env!("CARGO_PKG_VERSION"),
    " (https://github.com/turbinebmw/Campfe)"
);
