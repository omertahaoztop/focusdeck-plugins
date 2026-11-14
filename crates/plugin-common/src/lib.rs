use std::net::IpAddr;
use std::time::Duration;

use reqwest::Url;
use reqwest::blocking::{Client, ClientBuilder};
use reqwest::redirect::Policy;
use serde::{Deserialize, Deserializer};

pub fn null_as_default<'de, D, T>(d: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de> + Default,
{
    Ok(Option::<T>::deserialize(d)?.unwrap_or_default())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UrlError {
    Invalid,
    Insecure,
}

// plain http would put the credential on the wire, so it is only accepted for this machine
pub fn checked_url(raw: &str) -> Result<Url, UrlError> {
    let url = Url::parse(raw.trim()).map_err(|_| UrlError::Invalid)?;
    let Some(host) = url.host_str() else {
        return Err(UrlError::Invalid);
    };
    match url.scheme() {
        "https" => Ok(url),
        "http" if is_loopback(host) => Ok(url),
        "http" => Err(UrlError::Insecure),
        _ => Err(UrlError::Invalid),
    }
}

pub fn checked_base(raw: &str) -> Result<String, UrlError> {
    checked_url(raw)?;
    Ok(raw.trim().trim_end_matches('/').to_string())
}

fn is_loopback(host: &str) -> bool {
    host.eq_ignore_ascii_case("localhost")
        || host
            .trim_start_matches('[')
            .trim_end_matches(']')
            .parse::<IpAddr>()
            .is_ok_and(|ip| ip.is_loopback())
}

// a followed redirect would hand custom credential headers to whatever host the server names
pub fn http_client(user_agent: &'static str, timeout: Duration, targets: &[&str]) -> ClientBuilder {
    let builder = Client::builder()
        .user_agent(user_agent)
        .timeout(timeout)
        .redirect(Policy::none());
    // reqwest would send plain http through HTTP_PROXY or ALL_PROXY, credential included
    if targets.iter().any(|url| is_plain_http(url)) {
        builder.no_proxy()
    } else {
        builder
    }
}

fn is_plain_http(url: &str) -> bool {
    url.get(..5)
        .is_some_and(|scheme| scheme.eq_ignore_ascii_case("http:"))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transport {
    TimedOut,
    Connect,
    Other,
}

impl Transport {
    pub fn of(error: &reqwest::Error) -> Self {
        if error.is_timeout() {
            Self::TimedOut
        } else if error.is_connect() {
            Self::Connect
        } else {
            Self::Other
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusReason {
    Redirected,
    BadRequest,
    Unauthorized,
    Forbidden,
    NotFound,
    RateLimited,
    ServerError,
    Unexpected,
}

impl StatusReason {
    pub fn of(status: u16) -> Self {
        match status {
            300..=399 => Self::Redirected,
            400 => Self::BadRequest,
            401 => Self::Unauthorized,
            403 => Self::Forbidden,
            404 => Self::NotFound,
            429 => Self::RateLimited,
            500..=599 => Self::ServerError,
            _ => Self::Unexpected,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecretError {
    Unreadable,
    Empty,
}

pub fn read_secret(path: &str) -> Result<String, SecretError> {
    let mut secret = std::fs::read_to_string(path).map_err(|_| SecretError::Unreadable)?;
    trim_in_place(&mut secret);
    if secret.is_empty() {
        return Err(SecretError::Empty);
    }
    Ok(secret)
}

// remote text reaches the terminal, where control and bidi override characters can rewrite what the user sees
pub fn printable(text: &str) -> String {
    let mut out: String = text.chars().filter(|c| !is_hostile(*c)).collect();
    trim_in_place(&mut out);
    out
}

fn is_hostile(c: char) -> bool {
    c.is_control() || matches!(c, '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}')
}

fn trim_in_place(text: &mut String) {
    text.truncate(text.trim_end().len());
    let lead = text.len() - text.trim_start().len();
    text.drain(..lead);
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use proptest::prelude::*;

    use super::*;

    #[test]
    fn https_is_accepted_anywhere() {
        for raw in [
            "https://redmine.example.com",
            "https://redmine.example.com/sub/",
            " https://10.0.0.5:8443 ",
        ] {
            assert!(checked_url(raw).is_ok(), "{raw}");
        }
    }

    #[test]
    fn http_is_accepted_only_for_this_machine() {
        for raw in [
            "http://127.0.0.1:3000",
            "http://127.9.9.9",
            "http://localhost:8080/api",
            "http://LOCALHOST",
            "http://[::1]:8080",
        ] {
            assert!(checked_url(raw).is_ok(), "{raw}");
        }
        for raw in [
            "http://redmine.example.com",
            "http://192.168.1.10",
            "http://localhost.example.com",
            "http://127.0.0.1.example.com",
            "http://[::2]",
        ] {
            assert_eq!(checked_url(raw), Err(UrlError::Insecure), "{raw}");
        }
    }

    #[test]
    fn other_schemes_and_garbage_are_invalid() {
        for raw in [
            "",
            "redmine.example.com",
            "ftp://example.com",
            "file:///etc/passwd",
            "https://",
            "javascript:alert(1)",
        ] {
            assert_eq!(checked_url(raw), Err(UrlError::Invalid), "{raw}");
        }
    }

    #[test]
    fn base_drops_trailing_slashes() {
        assert_eq!(
            checked_base(" https://x.example.com/sub// ").unwrap(),
            "https://x.example.com/sub"
        );
        assert_eq!(
            checked_base("http://x.example.com"),
            Err(UrlError::Insecure)
        );
    }

    #[test]
    fn redirects_are_not_followed() {
        let mut server = mockito::Server::new();
        let elsewhere = server.mock("GET", "/elsewhere").expect(0).create();
        server
            .mock("GET", "/start")
            .with_status(302)
            .with_header("location", "/elsewhere")
            .create();
        let http = http_client("test/0", Duration::from_secs(5), &[])
            .build()
            .unwrap();
        let response = http.get(format!("{}/start", server.url())).send().unwrap();
        assert_eq!(response.status().as_u16(), 302);
        elsewhere.assert();
    }

    #[test]
    fn transport_errors_are_classified() {
        let http = http_client("test/0", Duration::from_secs(5), &[])
            .build()
            .unwrap();
        let closed = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = closed.local_addr().unwrap().port();
        drop(closed);
        let error = http
            .get(format!("http://127.0.0.1:{port}/"))
            .send()
            .unwrap_err();
        assert_eq!(Transport::of(&error), Transport::Connect);
    }

    #[test]
    fn status_reasons() {
        let cases = [
            (302, StatusReason::Redirected),
            (400, StatusReason::BadRequest),
            (401, StatusReason::Unauthorized),
            (403, StatusReason::Forbidden),
            (404, StatusReason::NotFound),
            (429, StatusReason::RateLimited),
            (500, StatusReason::ServerError),
            (503, StatusReason::ServerError),
            (418, StatusReason::Unexpected),
        ];
        for (status, reason) in cases {
            assert_eq!(StatusReason::of(status), reason, "{status}");
        }
    }

    #[test]
    fn secret_is_trimmed_and_must_not_be_empty() {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        writeln!(file, "  s3cret \n").unwrap();
        let path = file.path().to_str().unwrap();
        assert_eq!(read_secret(path).unwrap(), "s3cret");
        std::fs::write(file.path(), " \n\t").unwrap();
        assert_eq!(read_secret(path), Err(SecretError::Empty));
        assert_eq!(
            read_secret("/nonexistent/seenboard-secret"),
            Err(SecretError::Unreadable)
        );
    }

    #[test]
    fn null_becomes_default() {
        #[derive(Deserialize)]
        struct Row {
            #[serde(default, deserialize_with = "null_as_default")]
            name: String,
        }
        for body in [r#"{"name":null}"#, "{}"] {
            let row: Row = serde_json::from_str(body).unwrap();
            assert_eq!(row.name, "", "{body}");
        }
    }

    #[test]
    fn printable_strips_escape_sequences_and_bidi_overrides() {
        assert_eq!(printable("\u{1b}[2Jfatura\u{7}"), "[2Jfatura");
        assert_eq!(printable("iade\u{202e}fdp.exe"), "iadefdp.exe");
        assert_eq!(printable("\u{2066}a\u{2069} b"), "a b");
        assert_eq!(printable("  Çağrı   "), "Çağrı");
        assert_eq!(printable(" \u{202e} x"), "x");
    }

    proptest! {
        #[test]
        fn printable_output_is_clean(text in "\\PC*|.*") {
            let out = printable(&text);
            prop_assert!(!out.chars().any(is_hostile));
            prop_assert_eq!(out.trim(), out.as_str());
            prop_assert_eq!(printable(&out), out.clone());
        }

        #[test]
        fn checked_url_never_panics(raw in ".*") {
            let _ = checked_url(&raw);
        }
    }
}
