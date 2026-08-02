//! Subscriber login and stream entitlement.
//!
//! FRISKY gates the 128k and 320k mounts behind a subscription. The web player
//! checks `validate-streaming` first, then appends the token to the stream URL
//! as `?token=`; this mirrors that. The 96k mount needs none of it.
//!
//! Tokens are stored in the Secret Service (GNOME Keyring), never on disk.

use anyhow::{Context, Result};
use std::collections::HashMap;
use tracing::{debug, warn};

const KEYRING_ATTRIBUTE: (&str, &str) = ("xdg:schema", "io.github.eboye.Frisky.Token");
const KEYRING_LABEL: &str = "FRISKY subscriber token";

fn attributes() -> HashMap<&'static str, &'static str> {
    HashMap::from([KEYRING_ATTRIBUTE])
}

/// Reads the stored token, if there is one.
///
/// A locked or absent keyring is not an error — the app simply stays on the
/// free tier.
pub async fn stored_token() -> Option<String> {
    match load().await {
        Ok(token) => token,
        Err(error) => {
            debug!("could not read keyring: {error:#}");
            None
        }
    }
}

async fn load() -> Result<Option<String>> {
    let keyring = oo7::Keyring::new().await.context("opening keyring")?;
    let items = keyring.search_items(&attributes()).await?;

    let Some(item) = items.first() else {
        return Ok(None);
    };
    let secret = item.secret().await?;
    let token = String::from_utf8(secret.to_vec()).context("token is not valid UTF-8")?;
    Ok((!token.is_empty()).then_some(token))
}

/// Replaces any stored token with `token`.
pub async fn store_token(token: &str) -> Result<()> {
    let keyring = oo7::Keyring::new().await.context("opening keyring")?;
    keyring
        .create_item(KEYRING_LABEL, &attributes(), token, true)
        .await
        .context("writing token to keyring")?;
    Ok(())
}

/// Forgets the stored token. Succeeds even if there was nothing to remove.
pub async fn clear_token() -> Result<()> {
    let keyring = oo7::Keyring::new().await.context("opening keyring")?;
    keyring
        .delete(&attributes())
        .await
        .context("removing token from keyring")?;
    Ok(())
}

/// Whether a token is good for a given channel and quality.
///
/// Anything unexpected answers "no", so a failure here downgrades to the free
/// stream rather than producing a 401 the user cannot interpret.
pub async fn is_entitled(
    client: &crate::api::FriskyClient,
    token: &str,
    channel: crate::channel::Channel,
    quality: crate::channel::Quality,
) -> bool {
    if !quality.requires_subscription() {
        return true;
    }
    match client.validate_stream(token, channel, quality).await {
        Ok(allowed) => allowed,
        Err(error) => {
            warn!("stream validation failed: {error:#}");
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keyring_attributes_are_namespaced_to_this_app() {
        let attributes = attributes();
        assert_eq!(
            attributes.get("xdg:schema"),
            Some(&"io.github.eboye.Frisky.Token")
        );
    }
}
