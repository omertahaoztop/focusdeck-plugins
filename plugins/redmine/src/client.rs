use std::collections::HashSet;
use std::time::Duration;

use reqwest::blocking::{Client as Http, Response};
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use seenboard_plugin_common::{StatusReason, Transport, http_client, null_as_default};
use serde::Deserialize;

use crate::i18n::{msg, t, t_args};

const PAGE_LIMIT: &str = "100";

#[derive(Debug, Clone, Deserialize)]
pub struct Priority {
    #[serde(default, deserialize_with = "null_as_default")]
    pub name: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Issue {
    pub id: u64,
    #[serde(default, deserialize_with = "null_as_default")]
    pub subject: String,
    #[serde(default)]
    pub priority: Option<Priority>,
    #[serde(default)]
    pub due_date: Option<String>,
}

#[derive(Deserialize)]
struct Page {
    issues: Vec<Issue>,
    total_count: usize,
}

pub struct Client {
    http: Http,
    base: String,
}

impl Client {
    pub fn new(base: &str, key: &str) -> Result<Self, String> {
        let mut value = HeaderValue::from_str(key).map_err(|_| t(msg::KEY_INVALID_CHARS))?;
        value.set_sensitive(true);
        let mut headers = HeaderMap::new();
        headers.insert(HeaderName::from_static("x-redmine-api-key"), value);
        let http = http_client(
            concat!("seenboard-redmine/", env!("CARGO_PKG_VERSION")),
            Duration::from_secs(15),
            &[base],
        )
        .default_headers(headers)
        .build()
        .map_err(|_| t(msg::HTTP_CLIENT_FAILED))?;
        Ok(Self {
            http,
            base: base.to_string(),
        })
    }

    fn get(&self, path: &str, query: &[(&str, &str)]) -> Result<Response, String> {
        let response = self
            .http
            .get(format!("{}{path}", self.base))
            .query(query)
            .send()
            .map_err(|e| transport_error(&e))?;
        match response.status() {
            status if status.is_success() => Ok(response),
            status => Err(status_error(status.as_u16())),
        }
    }

    pub fn current_user(&self) -> Result<(), String> {
        self.get("/users/current.json", &[]).map(drop)
    }

    // pages advance by what the server actually returned, since it may cap limit below ours
    pub fn issues(&self) -> Result<Vec<Issue>, String> {
        let mut all: Vec<Issue> = Vec::new();
        let mut seen = HashSet::new();
        let mut offset = 0;
        let total = loop {
            let offset_text = offset.to_string();
            let query = [
                ("assigned_to_id", "me"),
                ("status_id", "open"),
                ("sort", "priority:desc,updated_on:desc"),
                ("limit", PAGE_LIMIT),
                ("offset", offset_text.as_str()),
            ];
            let page: Page = self
                .get("/issues.json", &query)?
                .json()
                .map_err(|_| t(msg::RESPONSE_UNREADABLE))?;
            if page.issues.is_empty() {
                break page.total_count;
            }
            offset += page.issues.len();
            all.extend(page.issues.into_iter().filter(|i| seen.insert(i.id)));
            if offset >= page.total_count {
                break page.total_count;
            }
        };
        if all.len() < total {
            return Err(t_args(
                msg::LIST_INCOMPLETE,
                &[("got", all.len().into()), ("total", total.into())],
            ));
        }
        Ok(all)
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
