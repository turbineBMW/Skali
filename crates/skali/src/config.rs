pub const APP_ID: &str = "io.github.turbinebmw.Skali";
pub const RESOURCE_PATH: &str = "/io/github/turbinebmw/Skali";
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
/// The schemas compiled by build.rs, used when the app isn't installed.
pub const BUILT_SCHEMA_DIR: &str = concat!(env!("OUT_DIR"), "/schemas");
pub const GETTEXT_DOMAIN: &str = "skali";

/// The Launchpad integration Skali signs in as. These are the public
/// credentials of a native app, as 37signals' own CLI ships its own: they
/// identify the app, not a user, and the redirect is pinned to the
/// loopback port, so they open nothing on their own. `SKALI_OAUTH_CLIENT_ID`
/// and `SKALI_OAUTH_CLIENT_SECRET`, at build or run time, swap in another
/// integration.
pub const OAUTH_CLIENT_ID: &str = match option_env!("SKALI_OAUTH_CLIENT_ID") {
    Some(id) => id,
    None => "426dcf4e3de7cd87e80e04b7840643e0ad8493b9",
};
pub const OAUTH_CLIENT_SECRET: &str = match option_env!("SKALI_OAUTH_CLIENT_SECRET") {
    Some(secret) => secret,
    None => "5ee7c953eb55c614fa4bf2a9f92b3a83c0ba873f",
};
/// Registered with the integration; must match exactly.
pub const OAUTH_REDIRECT_URI: &str = "http://127.0.0.1:8976/callback";

pub fn oauth_app() -> skali_core::auth::OAuthApp {
    skali_core::auth::OAuthApp {
        client_id: std::env::var("SKALI_OAUTH_CLIENT_ID")
            .unwrap_or_else(|_| OAUTH_CLIENT_ID.into()),
        client_secret: std::env::var("SKALI_OAUTH_CLIENT_SECRET")
            .unwrap_or_else(|_| OAUTH_CLIENT_SECRET.into()),
        redirect_uri: OAUTH_REDIRECT_URI.into(),
    }
}
