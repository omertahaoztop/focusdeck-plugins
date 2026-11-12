use std::collections::HashSet;
use std::time::Duration;

use focusdeck_plugin_common::{StatusReason, Transport, http_client, null_as_default};
use reqwest::blocking::{Client as Http, RequestBuilder, Response};
use serde::Deserialize;
use serde_json::Value;

use crate::i18n::{msg, t, t_args};

const BATCH: &str = "1000";
const MAX_PAGES: usize = 100;
const STARRED_NARROW: &str = r#"[{"operator":"is","operand":"starred"}]"#;

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
    pub fn new(base: &str, email: &str, key: &str) -> Result<Self, String> {
        let http = http_client(
            concat!("focusdeck-zulip/", env!("CARGO_PKG_VERSION")),
            Duration::from_secs(15),
            &[base],
        )
        .build()
        .map_err(|_| t(msg::HTTP_CLIENT_FAILED))?;
        Ok(Self {
            http,
            base: base.to_string(),
            email: email.to_string(),
            key: key.to_string(),
        })
    }

    fn send(&self, request: RequestBuilder) -> Result<Response, String> {
        let response = request
            .basic_auth(&self.email, Some(&self.key))
            .send()
            .map_err(|e| transport_error(&e))?;
        match response.status() {
            status if status.is_success() => Ok(response),
            status => Err(status_error(status.as_u16())),
        }
    }

    pub fn me(&self) -> Result<(), String> {
        self.send(self.http.get(format!("{}/api/v1/users/me", self.base)))
            .map(drop)
    }

    // pages walk backwards from the newest message until the server reports the oldest one
    pub fn starred(&self) -> Result<Vec<Message>, String> {
        let mut all: Vec<Message> = Vec::new();
        let mut seen = HashSet::new();
        let mut anchor = "newest".to_string();
        for _ in 0..MAX_PAGES {
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
                .send(request)?
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
        self.send(request).map(drop)
    }
}

fn transport_error(error: &reqwest::Error) -> String {
    t(match Transport::of(error) {
        Transport::TimedOut => msg::TIMED_OUT,
        Transport::Connect => msg::CONNECT_FAILED,
        Transport::Other => msg::REQUEST_FAILED,
    })
}

fn status_error(status: u16) -> String {
    let reason = match StatusReason::of(status) {
        StatusReason::Redirected => msg::HTTP_REDIRECTED,
        StatusReason::BadRequest => msg::HTTP_BAD_REQUEST,
        StatusReason::Unauthorized => msg::HTTP_UNAUTHORIZED,
        StatusReason::Forbidden => msg::HTTP_FORBIDDEN,
        StatusReason::NotFound => msg::HTTP_NOT_FOUND,
        StatusReason::RateLimited => msg::HTTP_RATE_LIMITED,
        StatusReason::ServerError => msg::HTTP_SERVER_ERROR,
        StatusReason::Unexpected => msg::HTTP_UNEXPECTED,
    };
    t_args(
        msg::HTTP_STATUS,
        &[
            ("status", i64::from(status).into()),
            ("reason", t(reason).as_str().into()),
        ],
    )
}
