use std::collections::HashSet;
use std::time::Duration;

use chrono::{DateTime, Utc};
use focusdeck_plugin_common::{StatusReason, Transport, http_client, null_as_default};
use reqwest::Url;
use reqwest::blocking::{Client as Http, RequestBuilder, Response};
use serde::Deserialize;
use serde_json::json;

use crate::i18n::{msg, t, t_args};

const LIMIT: usize = 100;
const MAX_PAGES: usize = 20;

#[derive(Debug, Clone, Deserialize)]
pub struct Message {
    pub id: String,
    #[serde(default, deserialize_with = "null_as_default")]
    pub content: String,
    pub timestamp: DateTime<Utc>,
    #[serde(default, deserialize_with = "null_as_default")]
    pub is_from_me: bool,
    #[serde(default, deserialize_with = "null_as_default")]
    pub media_type: String,
    #[serde(default, deserialize_with = "null_as_default")]
    pub filename: String,
}

#[derive(Deserialize, Default)]
struct Pagination {
    #[serde(default, deserialize_with = "null_as_default")]
    total: usize,
}

#[derive(Deserialize, Default)]
struct Results {
    #[serde(default, deserialize_with = "null_as_default")]
    data: Vec<Message>,
    #[serde(default, deserialize_with = "null_as_default")]
    pagination: Pagination,
}

#[derive(Deserialize)]
struct Page {
    #[serde(default, deserialize_with = "null_as_default")]
    results: Results,
}

pub struct Client {
    http: Http,
    base: Url,
    endpoint: String,
    jid: String,
    user: String,
    password: String,
    since: Option<DateTime<Utc>>,
}

impl Client {
    pub fn new(
        base: &str,
        user: &str,
        password: &str,
        jid: &str,
        since: Option<DateTime<Utc>>,
    ) -> Result<Self, String> {
        let http = http_client(
            concat!("focusdeck-whatsapp/", env!("CARGO_PKG_VERSION")),
            Duration::from_secs(15),
            &[base],
        )
        .build()
        .map_err(|_| t(msg::HTTP_CLIENT_FAILED))?;
        Ok(Self {
            http,
            base: Url::parse(base).map_err(|_| t(msg::URL_INVALID))?,
            endpoint: format!("{base}/chat/{}/messages", jid.replace('@', "%40")),
            jid: jid.to_string(),
            user: user.to_string(),
            password: password.to_string(),
            since,
        })
    }

    fn fetch(&self, limit: usize, offset: usize) -> Result<Results, String> {
        let mut query = vec![("limit", limit.to_string()), ("offset", offset.to_string())];
        if let Some(since) = self.since {
            query.push(("start_time", since.to_rfc3339()));
        }
        let request: RequestBuilder = self.http.get(&self.endpoint).query(&query);
        let page: Page = self
            .send(request)?
            .json()
            .map_err(|_| t(msg::RESPONSE_UNREADABLE))?;
        Ok(page.results)
    }

    fn send(&self, request: RequestBuilder) -> Result<Response, String> {
        let response = request
            .basic_auth(&self.user, Some(&self.password))
            .send()
            .map_err(|e| transport_error(&e))?;
        match response.status() {
            status if status.is_success() => Ok(response),
            status => Err(status_error(status.as_u16())),
        }
    }

    pub fn probe(&self) -> Result<(), String> {
        self.fetch(1, 0).map(drop)
    }

    // delete-for-me rather than revoke, which would leave a "message deleted" stub in the chat
    pub fn delete(&self, id: &str) -> Result<(), String> {
        // the url crate drops "." and ".." segments, which would post to another path
        if matches!(id, "" | "." | "..") {
            return Err(t(msg::MESSAGE_ID_INVALID));
        }
        let mut url = self.base.clone();
        url.path_segments_mut()
            .map_err(|()| t(msg::URL_INVALID))?
            .pop_if_empty()
            .extend(["message", id, "delete"]);
        let request = self.http.post(url).json(&json!({"phone": self.jid}));
        self.send(request).map(drop)
    }

    // an incomplete walk is an error so a short list never passes for the whole chat
    pub fn messages(&self) -> Result<Vec<Message>, String> {
        let mut all: Vec<Message> = Vec::new();
        let mut seen = HashSet::new();
        let mut offset = 0;
        let mut total = 0;
        let mut complete = false;
        let mut repeated = false;
        // one probe page past the cap tells a chat of exactly MAX_PAGES * LIMIT rows from a longer one
        for page in 0..=MAX_PAGES {
            let results = self.fetch(LIMIT, offset)?;
            total = results.pagination.total;
            let rows = results.data.len();
            if page == MAX_PAGES {
                complete = rows == 0;
                break;
            }
            offset += rows;
            for m in results.data {
                if seen.insert(m.id.clone()) {
                    all.push(m);
                } else {
                    repeated = true;
                }
            }
            if repeated || rows < LIMIT {
                complete = rows < LIMIT;
                break;
            }
        }
        // total counts the whole chat and ignores start_time, so it only checks an unfiltered walk
        let counted = self.since.is_some() || all.len() == total;
        if !complete || repeated || !counted {
            return Err(t_args(
                msg::LISTING_INCOMPLETE,
                &[
                    ("got", (all.len() as i64).into()),
                    ("total", (total as i64).into()),
                ],
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
        StatusReason::Unauthorized => return t(msg::PASSWORD_REJECTED),
        StatusReason::NotFound => return t(msg::ADDRESS_NOT_FOUND),
        StatusReason::Redirected => msg::HTTP_REDIRECTED,
        StatusReason::BadRequest => msg::HTTP_BAD_REQUEST,
        StatusReason::Forbidden => msg::HTTP_FORBIDDEN,
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
