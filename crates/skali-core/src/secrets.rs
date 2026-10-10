//! The keyring: the tokens of each Launchpad identity, as one JSON secret
//! in the desktop's Secret Service (or the Secret portal in a sandbox).
//! Called from the tokio runtime; the keyring is reached over D-Bus.

use crate::auth::TokenSet;
use crate::error::{ApiError, Result};
use std::collections::HashMap;

const SCHEMA: &str = "io.github.turbinebmw.Skali.Token";
/// The schema of the app's first name, Campfe; an item under it moves to
/// `SCHEMA` the first time it's read.
const OLD_SCHEMA: &str = "io.github.turbinebmw.Campfe.Token";

fn attributes(identity_id: i64) -> HashMap<&'static str, String> {
    attributes_in(SCHEMA, identity_id)
}

fn attributes_in(schema: &str, identity_id: i64) -> HashMap<&'static str, String> {
    HashMap::from([
        ("xdg:schema", schema.to_string()),
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
            &format!("Skali: {label}"),
            &attributes(identity_id),
            secret,
            true,
        )
        .await
        .map_err(keyring_error)?;
    Ok(())
}

pub async fn load_tokens(identity_id: i64) -> Result<Option<TokenSet>> {
    let keyring = keyring().await?;
    let items = keyring
        .search_items(&attributes(identity_id))
        .await
        .map_err(keyring_error)?;
    if let Some(item) = items.first() {
        let secret = item.secret().await.map_err(keyring_error)?;
        return Ok(Some(serde_json::from_slice(&secret)?));
    }
    // Stored by Campfe: copy it to Skali's schema, then drop the old item.
    let old = keyring
        .search_items(&attributes_in(OLD_SCHEMA, identity_id))
        .await
        .map_err(keyring_error)?;
    let Some(item) = old.first() else {
        return Ok(None);
    };
    let secret = item.secret().await.map_err(keyring_error)?;
    let tokens: TokenSet = serde_json::from_slice(&secret)?;
    let label = item.label().await.map_err(keyring_error)?;
    let label = label.strip_prefix("Campfe: ").unwrap_or(&label).to_string();
    store_tokens(identity_id, &label, &tokens).await?;
    item.delete().await.map_err(keyring_error)?;
    tracing::info!("moved the tokens of identity {identity_id} to Skali's keyring schema");
    Ok(Some(tokens))
}

pub async fn clear_tokens(identity_id: i64) -> Result<()> {
    let keyring = keyring().await?;
    for schema in [SCHEMA, OLD_SCHEMA] {
        keyring
            .delete(&attributes_in(schema, identity_id))
            .await
            .map_err(keyring_error)?;
    }
    Ok(())
}
