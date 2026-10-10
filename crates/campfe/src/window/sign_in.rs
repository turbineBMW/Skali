//! Signing in: the browser goes to Launchpad, comes back to our loopback
//! port with a code, and the tokens land in the keyring with the accounts
//! they open in the cache. Also the token refresh, on the same identity.

use super::{CampfeWindow, PAGE_MAIN, PAGE_NO_ACCOUNT, PAGE_SIGNING_IN};
use crate::config;
use crate::i18n::gettext;
use crate::rt;
use adw::subclass::prelude::*;
use campfe_core::auth::{self, OAuthApp, TokenSet};
use campfe_core::error::{ApiError, Result};
use campfe_core::models::{Account, Authorization};
use gtk::glib;

/// What a finished sign-in brings back.
pub struct SignedIn {
    pub tokens: TokenSet,
    pub authorization: Authorization,
}

impl CampfeWindow {
    pub fn start_sign_in(&self) {
        if self.state().is_signing_in {
            return;
        }
        let app = config::oauth_app();
        if !app.is_configured() {
            self.toast(&gettext(
                "No Launchpad integration is built in. Set CAMPFE_OAUTH_CLIENT_ID and CAMPFE_OAUTH_CLIENT_SECRET.",
            ));
            return;
        }
        let state = glib::uuid_string_random().to_string();
        let url = app.authorize_url(&state);
        self.state_mut().is_signing_in = true;
        self.imp()
            .main_stack
            .set_visible_child_name(PAGE_SIGNING_IN);

        // Listen first, then open the browser, so the redirect can't beat us.
        rt::run(
            async move { sign_in(app, state).await },
            glib::clone!(
                #[weak(rename_to = window)]
                self,
                move |result| window.sign_in_finished(result)
            ),
        );
        gtk::UriLauncher::new(&url).launch(
            Some(self),
            gio::Cancellable::NONE,
            glib::clone!(
                #[weak(rename_to = window)]
                self,
                move |result| {
                    if let Err(error) = result {
                        tracing::error!("could not open the browser: {error}");
                        window.toast(&gettext("Could not open a browser to sign in with."));
                    }
                }
            ),
        );
    }

    fn sign_in_finished(&self, result: Result<SignedIn>) {
        self.state_mut().is_signing_in = false;
        let signed_in = match result {
            Ok(signed_in) => signed_in,
            Err(error) => {
                self.report("sign-in failed", &error);
                let page = if self.state().accounts.is_empty() {
                    PAGE_NO_ACCOUNT
                } else {
                    PAGE_MAIN
                };
                self.imp().main_stack.set_visible_child_name(page);
                return;
            }
        };
        let identity = signed_in.authorization.identity;
        let accounts: Vec<Account> = signed_in
            .authorization
            .accounts
            .iter()
            .filter(|a| a.is_basecamp())
            .map(|a| Account {
                id: a.id,
                name: a.name.clone(),
                href: a.href.clone(),
                app_href: a.app_href.clone(),
                identity_id: identity.id,
                identity_name: identity.full_name(),
                identity_email: identity.email_address.clone(),
            })
            .collect();
        if accounts.is_empty() {
            self.toast(&gettext("This sign-in has no Basecamp account."));
            self.imp()
                .main_stack
                .set_visible_child_name(PAGE_NO_ACCOUNT);
            return;
        }
        {
            let db = self.db();
            let db = db.borrow();
            for account in &accounts {
                if let Err(error) = db.upsert_account(account) {
                    self.report("could not save the account", &error);
                }
            }
        }
        {
            let mut state = self.state_mut();
            for account in accounts {
                state.accounts.insert(account.id, account);
            }
        }
        let label = format!("{} <{}>", identity.full_name(), identity.email_address);
        let tokens = signed_in.tokens.clone();
        let identity_id = identity.id;
        rt::run(
            async move { campfe_core::secrets::store_tokens(identity_id, &label, &tokens).await },
            glib::clone!(
                #[weak(rename_to = window)]
                self,
                move |result| {
                    if let Err(error) = result {
                        window.report("could not store the tokens in the keyring", &error);
                    }
                }
            ),
        );
        self.imp().main_stack.set_visible_child_name(PAGE_MAIN);
        self.toast(&gettext("Signed in to Basecamp."));
        self.tokens_loaded(identity_id, signed_in.tokens);
    }

    /// A fresh access token for an identity, written into the shared token
    /// every client of its accounts reads, and into the keyring.
    pub(super) fn refresh_tokens(&self, identity_id: i64, tokens: TokenSet) {
        let app = config::oauth_app();
        rt::run(
            async move {
                let http = auth::launchpad_client()?;
                auth::refresh(&http, &app, &tokens).await
            },
            glib::clone!(
                #[weak(rename_to = window)]
                self,
                move |result| match result {
                    Ok(fresh) => window.tokens_refreshed(identity_id, fresh),
                    Err(error) => window.report("could not refresh the sign-in", &error),
                }
            ),
        );
    }

    fn tokens_refreshed(&self, identity_id: i64, fresh: TokenSet) {
        let shared = self.state().tokens.get(&identity_id).cloned();
        let access_token = fresh.access_token.clone();
        let label = self
            .state()
            .accounts
            .values()
            .find(|a| a.identity_id == identity_id)
            .map(|a| format!("{} <{}>", a.identity_name, a.identity_email))
            .unwrap_or_else(|| identity_id.to_string());
        rt::run(
            async move {
                if let Some(shared) = shared {
                    *shared.write().await = access_token;
                }
                campfe_core::secrets::store_tokens(identity_id, &label, &fresh).await
            },
            glib::clone!(
                #[weak(rename_to = window)]
                self,
                move |result| {
                    if let Err(error) = result {
                        window.report("could not store the refreshed tokens", &error);
                    }
                }
            ),
        );
    }
}

/// The whole dance, off the main thread: wait for the browser's redirect,
/// exchange the code, ask Launchpad whose token it is.
async fn sign_in(app: OAuthApp, state: String) -> Result<SignedIn> {
    let port = app
        .redirect_port()
        .ok_or_else(|| ApiError::Auth("the redirect URI has no port".into()))?;
    let code = tokio::time::timeout(
        std::time::Duration::from_secs(300),
        auth::await_callback(port, &state),
    )
    .await
    .map_err(|_| ApiError::Auth(gettext("no answer from the browser in five minutes")))??;
    let http = auth::launchpad_client()?;
    let tokens = auth::exchange_code(&http, &app, &code).await?;
    let authorization = auth::authorization(&http, &tokens.access_token).await?;
    Ok(SignedIn {
        tokens,
        authorization,
    })
}
