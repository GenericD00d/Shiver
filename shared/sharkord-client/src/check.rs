//! The add-server check both clients run before storing anything.

use shiver_core::{
    login, normalize_origin,
    probe::{self, ServerInfo},
};
use zeroize::Zeroizing;

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerCheck {
    #[serde(flatten)]
    pub info: ServerInfo,
    /// Absent when unchecked (no credentials); `None` when checked and not installed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plugin: Option<Option<String>>,
}

/// `/info`, plus — given credentials — whether the companion plugin is installed, which only the
/// join payload of a signed-in connection reveals. Nothing is stored.
pub async fn check_server(
    origin: &str,
    identity: Option<&str>,
    password: Option<&str>,
) -> shiver_core::Result<ServerCheck> {
    let origin = normalize_origin(origin)?;
    let info = probe::fetch_info(&origin).await?;

    let (Some(identity), Some(password)) = (
        identity
            .map(str::trim)
            .filter(|identity| !identity.is_empty()),
        password.filter(|password| !password.is_empty()),
    ) else {
        return Ok(ServerCheck { info, plugin: None });
    };

    let token = Zeroizing::new(login::sign_in(&origin, identity, password).await?);
    let session = crate::open(&origin, &token, false)
        .await
        .map_err(|error| shiver_core::Error::Unreachable(format!("{origin}: {error}")))?;

    Ok(ServerCheck {
        info,
        plugin: Some(session.joined.plugin_version.clone()),
    })
}
