//! The Google OAuth client (PLAN.md D25): never in the repo. From
//! `FAST_HARNESS_GOOGLE_CLIENT_ID` and `FAST_HARNESS_GOOGLE_CLIENT_SECRET`,
//! else `<app home>/google-client.json`:
//! `{"version":1,"id":"<client id>","secret":"<base64 of platform::protect(secret)>"}`,
//! the secret encrypted like account secrets.

use super::GoogleError;
use crate::accounts::crypto::{base64_decode, base64_encode};
use crate::platform;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// The client id variable.
pub const ENV_ID: &str = "FAST_HARNESS_GOOGLE_CLIENT_ID";
/// The client secret variable.
pub const ENV_SECRET: &str = "FAST_HARNESS_GOOGLE_CLIENT_SECRET";
/// The file name under the app home.
pub const CLIENT_FILE: &str = "google-client.json";
/// The schema version of [`CLIENT_FILE`].
pub const CLIENT_VERSION: u64 = 1;

/// An OAuth client. `Debug` hides both values.
#[derive(Clone, PartialEq, Eq)]
pub struct GoogleClient {
    pub id: String,
    pub secret: String,
}

impl fmt::Debug for GoogleClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GoogleClient")
            .field("id", &"<redacted>")
            .field("secret", &"<redacted>")
            .finish()
    }
}

#[derive(Serialize, Deserialize)]
struct ClientFile {
    version: u64,
    id: String,
    secret: String,
}

/// `<app_home>/google-client.json`.
pub fn client_path(app_home: &Path) -> PathBuf {
    app_home.join(CLIENT_FILE)
}

/// The client from the environment, else from `<app_home>/google-client.json`.
pub fn load_client(app_home: &Path) -> Result<GoogleClient, GoogleError> {
    let var = |name| std::env::var(name).ok();
    client_from(var(ENV_ID), var(ENV_SECRET), app_home)
}

/// [`load_client`] with the variables given: both set and non-empty win,
/// else the file; neither is [`GoogleError::NoClient`].
pub fn client_from(
    env_id: Option<String>,
    env_secret: Option<String>,
    app_home: &Path,
) -> Result<GoogleClient, GoogleError> {
    let filled = |v: Option<String>| v.map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    if let (Some(id), Some(secret)) = (filled(env_id), filled(env_secret)) {
        return Ok(GoogleClient { id, secret });
    }
    let text = match fs::read_to_string(client_path(app_home)) {
        Ok(text) => text,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Err(GoogleError::NoClient),
        Err(err) => return Err(GoogleError::Client(format!("{CLIENT_FILE}: {err}"))),
    };
    let file: ClientFile = serde_json::from_str(&text)
        .map_err(|_| GoogleError::Client(format!("{CLIENT_FILE} is not valid")))?;
    if file.version != CLIENT_VERSION {
        let version = file.version;
        return Err(GoogleError::Client(format!(
            "{CLIENT_FILE} version {version} is not supported"
        )));
    }
    let unreadable = || GoogleError::Client(format!("the secret in {CLIENT_FILE} cannot be read"));
    let blob = base64_decode(&file.secret).map_err(|_| unreadable())?;
    let mut plain = platform::unprotect(&blob).map_err(|_| unreadable())?;
    let secret = String::from_utf8(plain.clone()).map_err(|_| unreadable());
    plain.fill(0);
    let id = file.id.trim().to_string();
    if id.is_empty() {
        return Err(GoogleError::Client(format!("{CLIENT_FILE} has no id")));
    }
    Ok(GoogleClient {
        id,
        secret: secret?,
    })
}

/// Writes `<app_home>/google-client.json` (temporary file, then rename) with
/// the secret encrypted.
pub fn save_client(app_home: &Path, id: &str, secret: &str) -> Result<(), GoogleError> {
    let (id, secret) = (id.trim(), secret.trim());
    if id.is_empty() || secret.is_empty() {
        return Err(GoogleError::Client(
            "the id and the secret must not be empty".into(),
        ));
    }
    let blob = platform::protect(secret.as_bytes())?;
    let file = ClientFile {
        version: CLIENT_VERSION,
        id: id.to_string(),
        secret: base64_encode(&blob),
    };
    let mut text = serde_json::to_string_pretty(&file)
        .map_err(|_| GoogleError::Client("cannot encode the file".into()))?;
    text.push('\n');
    fs::create_dir_all(app_home)?;
    let path = client_path(app_home);
    let tmp = path.with_file_name(format!("{CLIENT_FILE}.{}.tmp", std::process::id()));
    if let Err(err) = fs::write(&tmp, text).and_then(|()| fs::rename(&tmp, &path)) {
        let _ = fs::remove_file(&tmp);
        return Err(err.into());
    }
    Ok(())
}
