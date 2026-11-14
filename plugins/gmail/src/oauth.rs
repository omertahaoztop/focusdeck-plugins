use std::fs::{self, DirBuilder, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use reqwest::Url;
use reqwest::blocking::Client as Http;
use seenboard_plugin_common::checked_url;
use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::client::{status_error, transport_error, url_error};
use crate::i18n::{msg, t, t_args};

pub struct ClientSecret {
    pub client_id: String,
    pub client_secret: String,
    pub auth_uri: Url,
    pub token_uri: Url,
    pub desktop: bool,
}

#[derive(Deserialize)]
struct RawClient {
    client_id: String,
    client_secret: String,
    auth_uri: String,
    token_uri: String,
}

#[derive(Deserialize)]
struct ClientFile {
    installed: Option<RawClient>,
    web: Option<RawClient>,
}

#[derive(Deserialize)]
struct TokenFile {
    refresh_token: String,
}

#[derive(Deserialize)]
pub struct TokenResponse {
    #[serde(default)]
    pub access_token: String,
    #[serde(default)]
    pub expires_in: u64,
    pub refresh_token: Option<String>,
}

pub enum TokenError {
    Status(u16),
    Other(String),
}

pub fn read_client_file(path: &Path) -> Result<ClientSecret, String> {
    let path_error = |id| t_args(id, &[("path", path.display().to_string().as_str().into())]);
    let text = fs::read_to_string(path).map_err(|_| path_error(msg::CLIENT_FILE_UNREADABLE))?;
    let file: ClientFile =
        serde_json::from_str(&text).map_err(|_| path_error(msg::CLIENT_FILE_INVALID))?;
    let (raw, desktop) = match (file.installed, file.web) {
        (Some(raw), _) => (raw, true),
        (None, Some(raw)) => (raw, false),
        (None, None) => return Err(path_error(msg::CLIENT_FILE_INVALID)),
    };
    let uri = |field, raw: &str| checked_url(raw).map_err(|e| url_error(field, e));
    Ok(ClientSecret {
        auth_uri: uri("auth_uri", &raw.auth_uri)?,
        token_uri: uri("token_uri", &raw.token_uri)?,
        client_id: raw.client_id,
        client_secret: raw.client_secret,
        desktop,
    })
}

pub fn read_refresh_token(path: &Path) -> Result<String, String> {
    let shown = path.display();
    let text = fs::read_to_string(path).map_err(|_| {
        t_args(
            msg::KEY_FILE_UNREADABLE,
            &[("path", shown.to_string().as_str().into())],
        )
    })?;
    let file: TokenFile = serde_json::from_str(&text).map_err(|_| {
        t_args(
            msg::KEY_FILE_INVALID,
            &[("path", shown.to_string().as_str().into())],
        )
    })?;
    let token = file.refresh_token.trim();
    if token.is_empty() {
        return Err(t_args(
            msg::KEY_FILE_EMPTY,
            &[("path", shown.to_string().as_str().into())],
        ));
    }
    Ok(token.to_string())
}

pub fn pkce_challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

pub fn random_token(bytes: usize) -> Result<String, String> {
    let mut buf = vec![0u8; bytes];
    getrandom::fill(&mut buf).map_err(|_| t(msg::RANDOM_FAILED))?;
    Ok(URL_SAFE_NO_PAD.encode(buf))
}

pub fn request_token(
    http: &Http,
    secret: &ClientSecret,
    grant: &[(&str, &str)],
) -> Result<TokenResponse, TokenError> {
    let mut form = grant.to_vec();
    form.push(("client_id", &secret.client_id));
    form.push(("client_secret", &secret.client_secret));
    let response = http
        .post(secret.token_uri.clone())
        .form(&form)
        .send()
        .map_err(|e| TokenError::Other(transport_error(&e)))?;
    let status = response.status();
    if !status.is_success() {
        return Err(TokenError::Status(status.as_u16()));
    }
    response
        .json()
        .map_err(|_| TokenError::Other(t(msg::RESPONSE_UNREADABLE)))
}

pub fn token_error_message(error: TokenError) -> String {
    match error {
        TokenError::Status(code) => status_error(code),
        TokenError::Other(message) => message,
    }
}

pub fn write_token(path: &Path, refresh_token: &str) -> io::Result<()> {
    if let Some(parent) = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty() && !p.exists())
    {
        DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(parent)?;
    }
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    match fs::remove_file(&tmp) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e),
        _ => {}
    }
    let body = serde_json::json!({ "refresh_token": refresh_token }).to_string();
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&tmp)?;
        file.write_all(body.as_bytes())?;
        file.sync_all()?;
        fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use super::*;

    #[test]
    fn pkce_challenge_matches_rfc7636_vector() {
        assert_eq!(
            pkce_challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    #[test]
    fn write_token_creates_0600_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("token.json");
        write_token(&path, "r").unwrap();
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            r#"{"refresh_token":"r"}"#
        );
        assert!(!dir.path().join("token.json.tmp").exists());
    }

    #[test]
    fn write_token_creates_private_parent_and_replaces_stale_tmp() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested/gmail/token.json");
        write_token(&path, "first").unwrap();
        let parent = path.parent().unwrap();
        assert_eq!(
            std::fs::metadata(parent).unwrap().permissions().mode() & 0o777,
            0o700
        );
        let tmp = dir.path().join("nested/gmail/token.json.tmp");
        std::fs::write(&tmp, "stale").unwrap();
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o644)).unwrap();
        write_token(&path, "second").unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            r#"{"refresh_token":"second"}"#
        );
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert!(!tmp.exists());
    }

    #[test]
    fn random_tokens_are_unpadded_base64url_of_the_requested_size() {
        let a = random_token(32).unwrap();
        let b = random_token(32).unwrap();
        assert_eq!(a.len(), 43);
        assert_eq!(random_token(16).unwrap().len(), 22);
        assert_ne!(a, b);
        assert!(
            a.chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        );
    }

    #[test]
    fn client_file_accepts_installed_and_web() {
        let dir = tempfile::tempdir().unwrap();
        for key in ["installed", "web"] {
            let path = dir.path().join(format!("{key}.json"));
            let body = serde_json::json!({key: {
                "client_id": "id", "client_secret": "sec",
                "auth_uri": "https://a.example.com/auth", "token_uri": "https://a.example.com/token",
            }});
            std::fs::write(&path, body.to_string()).unwrap();
            let secret = read_client_file(&path).unwrap();
            assert_eq!(secret.client_id, "id");
            assert_eq!(secret.token_uri.as_str(), "https://a.example.com/token");
            assert_eq!(secret.desktop, key == "installed");
        }
    }

    #[test]
    fn client_file_errors_name_only_the_path() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("none.json");
        assert_eq!(
            read_client_file(&missing).err().unwrap(),
            format!("Gmail client file could not be read: {}", missing.display())
        );
        let bad = dir.path().join("bad.json");
        std::fs::write(&bad, r#"{"other":{"client_secret":"leaky"}}"#).unwrap();
        let err = read_client_file(&bad).err().unwrap();
        assert_eq!(
            err,
            format!("Gmail client file could not be parsed: {}", bad.display())
        );
        assert!(!err.contains("leaky"));
    }

    #[test]
    fn refresh_token_file_errors() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("token.json");
        std::fs::write(&path, r#"{"refresh_token":"  "}"#).unwrap();
        assert_eq!(
            read_refresh_token(&path).err().unwrap(),
            format!("Gmail key file is empty: {}", path.display())
        );
        std::fs::write(&path, r#"{"refresh_token":" r1 \n"}"#).unwrap();
        assert_eq!(read_refresh_token(&path).unwrap(), "r1");
        std::fs::write(&path, "not json").unwrap();
        assert_eq!(
            read_refresh_token(&path).err().unwrap(),
            format!("Gmail key file could not be parsed: {}", path.display())
        );
    }
}
