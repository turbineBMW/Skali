//! The keyring: the tokens of each Launchpad identity, as one JSON secret
//! in the desktop's Secret Service (or the Secret portal in a sandbox).
//! Called from the tokio runtime; the keyring is reached over D-Bus.

use crate::auth::TokenSet;
use crate::error::{ApiError, Result};
use std::collections::HashMap;

const SCHEMA: &str = "io.github.turbinebmw.Campfe.Token";

fn attributes(identity_id: i64) -> HashMap<&'static str, String> {
    HashMap::from([
        ("xdg:schema", SCHEMA.to_string()),
        ("identity-id", identity_id.to_string()),
    ])
}

fn keyring_error(error: oo7::Error) -> ApiError {
    ApiError::Keyring(error.to_string())
}

async fn keyring() -> Result<oo7::Keyring> {
    let keyring = oo7::Keyring::new().await.map_err(keyring_error)?;
    keyring.unlock().await.map_err(keyring_error)?;
    Ok(keyring)
}

pub async fn store_tokens(identity_id: i64, label: &str, tokens: &TokenSet) -> Result<()> {
    let secret = serde_json::to_vec(tokens)?;
    keyring()
        .await?
        .create_item(
            &format!("Campfe: {label}"),
            &attributes(identity_id),
            secret,
            true,
        )
        .await
        .map_err(keyring_error)?;
    Ok(())
}

pub async fn load_tokens(identity_id: i64) -> Result<Option<TokenSet>> {
    let items = keyring()
        .await?
        .search_items(&attributes(identity_id))
        .await
        .map_err(keyring_error)?;
    let Some(item) = items.first() else {
        return Ok(None);
    };
    let secret = item.secret().await.map_err(keyring_error)?;
    Ok(Some(serde_json::from_slice(&secret)?))
}

pub async fn clear_tokens(identity_id: i64) -> Result<()> {
    keyring()
        .await?
        .delete(&attributes(identity_id))
        .await
        .map_err(keyring_error)?;
    Ok(())
}
