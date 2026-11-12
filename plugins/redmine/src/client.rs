use std::collections::HashSet;
use std::time::Duration;

use reqwest::StatusCode;
use reqwest::blocking::{Client as Http, Response};
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use serde::{Deserialize, Deserializer};

use crate::i18n::{msg, t, t_args};

const PAGE_LIMIT: &str = "100";

fn null_as_default<'de, D, T>(d: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de> + Default,
{
    Ok(Option::<T>::deserialize(d)?.unwrap_or_default())
}

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
    pub fn new(base_url: &str, key: &str) -> Result<Self, String> {
        let mut value = HeaderValue::from_str(key).map_err(|_| t(msg::KEY_INVALID_CHARS))?;
        value.set_sensitive(true);
        let mut headers = HeaderMap::new();
        headers.insert(HeaderName::from_static("x-redmine-api-key"), value);
        let http = Http::builder()
            .default_headers(headers)
            .user_agent(concat!("focusdeck-redmine/", env!("CARGO_PKG_VERSION")))
            .timeout(Duration::from_secs(15))
            .build()
            .map_err(|_| t(msg::HTTP_CLIENT_FAILED))?;
        Ok(Self {
            http,
            base: base_url.trim_end_matches('/').to_string(),
        })
    }

    fn get(&self, path: &str, query: &[(&str, &str)]) -> Result<Response, String> {
        self.http
            .get(format!("{}{path}", self.base))
            .query(query)
            .send()
            .map_err(transport_error)
    }

    fn get_ok(&self, path: &str, query: &[(&str, &str)]) -> Result<Response, String> {
        let response = self.get(path, query)?;
        match response.status() {
            status if status.is_success() => Ok(response),
            status => Err(status_error(status)),
        }
    }

    pub fn current_user(&self) -> Result<(), String> {
        let response = self.get("/users/current.json", &[])?;
        match response.status() {
            StatusCode::UNAUTHORIZED => Err(t(msg::KEY_REJECTED)),
            status if status.is_success() => Ok(()),
            status => Err(status_error(status)),
        }
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
                .get_ok("/issues.json", &query)?
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
