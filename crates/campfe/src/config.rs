pub const APP_ID: &str = "io.github.turbinebmw.Campfe";
pub const RESOURCE_PATH: &str = "/io/github/turbinebmw/Campfe";
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
/// The schemas compiled by build.rs, used when the app isn't installed.
pub const BUILT_SCHEMA_DIR: &str = concat!(env!("OUT_DIR"), "/schemas");
pub const GETTEXT_DOMAIN: &str = "campfe";

/// The Launchpad integration Campfe signs in as. These are the public
/// credentials of a native app, as 37signals' own CLI ships its own: they
/// identify the app, not a user. Set at build time
/// (`CAMPFE_OAUTH_CLIENT_ID`/`_SECRET`); the same variables at run time
/// override, for a developer's own integration.
pub const OAUTH_CLIENT_ID: &str = match option_env!("CAMPFE_OAUTH_CLIENT_ID") {
    Some(id) => id,
    None => "",
};
pub const OAUTH_CLIENT_SECRET: &str = match option_env!("CAMPFE_OAUTH_CLIENT_SECRET") {
    Some(secret) => secret,
    None => "",
};
/// Registered with the integration; must match exactly.
pub const OAUTH_REDIRECT_URI: &str = "http://127.0.0.1:8976/callback";

pub fn oauth_app() -> campfe_core::auth::OAuthApp {
    campfe_core::auth::OAuthApp {
        client_id: std::env::var("CAMPFE_OAUTH_CLIENT_ID")
            .unwrap_or_else(|_| OAUTH_CLIENT_ID.into()),
        client_secret: std::env::var("CAMPFE_OAUTH_CLIENT_SECRET")
            .unwrap_or_else(|_| OAUTH_CLIENT_SECRET.into()),
        redirect_uri: OAUTH_REDIRECT_URI.into(),
    }
}
