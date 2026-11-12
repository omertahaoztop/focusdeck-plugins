use std::collections::HashSet;
use std::time::Duration;

use reqwest::StatusCode;
use reqwest::blocking::{Client as Http, RequestBuilder, Response};
use serde::{Deserialize, Deserializer};
use serde_json::Value;

use crate::i18n::{msg, t, t_args};

const BATCH: &str = "1000";
const STARRED_NARROW: &str = r#"[{"operator":"is","operand":"starred"}]"#;

fn null_as_default<'de, D, T>(d: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de> + Default,
{
    Ok(Option::<T>::deserialize(d)?.unwrap_or_default())
}

#[derive(Debug, Clone, Deserialize)]
pub struct Message {
    pub id: u64,
    #[serde(default, deserialize_with = "null_as_default")]
    pub content: String,
    #[serde(default, deserialize_with = "null_as_default")]
    pub sender_full_name: String,
    #[serde(default, rename = "type", deserialize_with = "null_as_default")]
    pub kind: String,
    #[serde(default)]
    pub display_recipient: Value,
    #[serde(default, deserialize_with = "null_as_default")]
    pub subject: String,
}

#[derive(Deserialize)]
struct Page {
    #[serde(default)]
    messages: Vec<Message>,
    #[serde(default)]
    found_oldest: bool,
}

pub struct Client {
    http: Http,
    base: String,
    email: String,
    key: String,
}

impl Client {
    pub fn new(base_url: &str, email: &str, key: &str) -> Result<Self, String> {
        let http = Http::builder()
            .user_agent(concat!("focusdeck-zulip/", env!("CARGO_PKG_VERSION")))
            .timeout(Duration::from_secs(15))
            .build()
            .map_err(|_| t(msg::HTTP_CLIENT_FAILED))?;
        Ok(Self {
            http,
            base: base_url.trim_end_matches('/').to_string(),
            email: email.to_string(),
            key: key.to_string(),
        })
    }

    fn send(&self, request: RequestBuilder) -> Result<Response, String> {
        request
            .basic_auth(&self.email, Some(&self.key))
            .send()
            .map_err(transport_error)
    }

    fn send_ok(&self, request: RequestBuilder) -> Result<Response, String> {
        let response = self.send(request)?;
        match response.status() {
            status if status.is_success() => Ok(response),
            status => Err(status_error(status)),
        }
    }

    pub fn me(&self) -> Result<(), String> {
        let response = self.send(self.http.get(format!("{}/api/v1/users/me", self.base)))?;
        match response.status() {
            StatusCode::UNAUTHORIZED => Err(t(msg::KEY_REJECTED)),
            status if status.is_success() => Ok(()),
            status => Err(status_error(status)),
        }
    }

    // pages walk backwards from the newest message until the server reports the oldest one
    pub fn starred(&self) -> Result<Vec<Message>, String> {
        let mut all: Vec<Message> = Vec::new();
        let mut seen = HashSet::new();
        let mut anchor = "newest".to_string();
        loop {
            let mut query = vec![
                ("anchor", anchor.as_str()),
                ("num_before", BATCH),
                ("num_after", "0"),
                ("apply_markdown", "false"),
                ("narrow", STARRED_NARROW),
            ];
            if anchor != "newest" {
                query.push(("include_anchor", "false"));
            }
            let request = self
                .http
                .get(format!("{}/api/v1/messages", self.base))
                .query(&query);
            let page: Page = self
                .send_ok(request)?
                .json()
                .map_err(|_| t(msg::RESPONSE_UNREADABLE))?;
            let oldest = page.messages.iter().map(|m| m.id).min();
            let before = all.len();
            all.extend(page.messages.into_iter().filter(|m| seen.insert(m.id)));
            match oldest {
                Some(id) if !page.found_oldest && all.len() > before => anchor = id.to_string(),
                _ => break,
            }
        }
        all.sort_by_key(|m| std::cmp::Reverse(m.id));
        Ok(all)
    }

    pub fn unstar(&self, id: u64) -> Result<(), String> {
        let messages = format!("[{id}]");
        let form = [
            ("messages", messages.as_str()),
            ("op", "remove"),
            ("flag", "starred"),
        ];
        let request = self
            .http
            .post(format!("{}/api/v1/messages/flags", self.base))
            .form(&form);
        self.send_ok(request).map(|_| ())
    }
}

fn transport_error(e: reqwest::Error) -> String {
    if e.is_timeout() {
        t(msg::TIMED_OUT)
    } else if e.is_connect() {
        t(msg::CONNECT_FAILED)
    } else {
        t(msg::REQUEST_FAILED)
    }
}

fn status_error(status: StatusCode) -> String {
    t_args(
        msg::HTTP_STATUS,
        &[("status", status.as_u16().to_string().as_str().into())],
    )
}
