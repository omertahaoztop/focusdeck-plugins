use std::collections::HashSet;
use std::time::{Duration, Instant};

use reqwest::Url;
use reqwest::blocking::{Client as Http, Response};
use serde::Deserialize;

use crate::i18n::{msg, t, t_args};
use crate::oauth::{ClientSecret, TokenError, request_token};

const RENEW_MARGIN: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    pub id: String,
    pub thread_id: String,
    pub internal_date_ms: i64,
    pub from: String,
    pub subject: String,
}

#[derive(Deserialize)]
struct IdPage {
    #[serde(default)]
    messages: Vec<IdRef>,
    #[serde(default, rename = "nextPageToken")]
    next_page_token: Option<String>,
}

#[derive(Deserialize)]
struct IdRef {
    id: String,
}

#[derive(Deserialize)]
struct Metadata {
    id: String,
    #[serde(default, rename = "threadId")]
    thread_id: String,
    #[serde(rename = "internalDate")]
    internal_date: String,
    #[serde(default)]
    payload: Payload,
}

#[derive(Deserialize, Default)]
struct Payload {
    #[serde(default)]
    headers: Vec<Header>,
}

#[derive(Deserialize)]
struct Header {
    name: String,
    value: String,
}

struct Access {
    token: String,
    expires_at: Instant,
}

pub struct Client {
    http: Http,
    api: String,
    secret: ClientSecret,
    refresh_token: String,
    access: Option<Access>,
}

impl Client {
    pub fn new(api_url: &str, secret: ClientSecret, refresh_token: String) -> Result<Self, String> {
        Ok(Self {
            http: http()?,
            api: api_url.trim_end_matches('/').to_string(),
            secret,
            refresh_token,
            access: None,
        })
    }

    pub fn authorize(&mut self) -> Result<(), String> {
        let grant = [
            ("grant_type", "refresh_token"),
            ("refresh_token", self.refresh_token.as_str()),
        ];
        let response = match request_token(&self.http, &self.secret, &grant) {
            Ok(response) => response,
            Err(TokenError::Status(400 | 401)) => return Err(t(msg::SESSION_LOST)),
            Err(TokenError::Status(code)) => return Err(status_error(code)),
            Err(TokenError::Other(message)) => return Err(message),
        };
        if response.access_token.is_empty() {
            return Err(t(msg::RESPONSE_UNREADABLE));
        }
        self.access = Some(Access {
            token: response.access_token,
            expires_at: Instant::now() + Duration::from_secs(response.expires_in),
        });
        Ok(())
    }

    fn bearer(&mut self) -> Result<String, String> {
        let fresh = self
            .access
            .as_ref()
            .is_some_and(|a| Instant::now() + RENEW_MARGIN < a.expires_at);
        if !fresh {
            self.authorize()?;
        }
        self.access
            .as_ref()
            .map(|a| a.token.clone())
            .ok_or_else(|| t(msg::RESPONSE_UNREADABLE))
    }

    fn get(&mut self, url: Url, query: &[(&str, &str)]) -> Result<Response, String> {
        let token = self.bearer()?;
        let response = self
            .http
            .get(url)
            .query(query)
            .bearer_auth(token)
            .send()
            .map_err(transport_error)?;
        match response.status() {
            status if status.is_success() => Ok(response),
            status => Err(status_error(status.as_u16())),
        }
    }

    fn url(&self, segments: &[&str]) -> Result<Url, String> {
        let mut url = Url::parse(&self.api).map_err(|_| t(msg::API_URL_INVALID))?;
        url.path_segments_mut()
            .map_err(|_| t(msg::API_URL_INVALID))?
            .pop_if_empty()
            .extend(segments);
        Ok(url)
    }

    pub fn starred(&mut self) -> Result<Vec<Message>, String> {
        let list_url = self.url(&["messages"])?;
        let mut ids: Vec<String> = Vec::new();
        let mut seen_ids = HashSet::new();
        let mut seen_tokens = HashSet::new();
        let mut page_token: Option<String> = None;
        loop {
            let mut query = vec![("q", "is:starred"), ("maxResults", "100")];
            if let Some(token) = &page_token {
                query.push(("pageToken", token));
            }
            let page: IdPage = self
                .get(list_url.clone(), &query)?
                .json()
                .map_err(|_| t(msg::RESPONSE_UNREADABLE))?;
            ids.extend(
                page.messages
                    .into_iter()
                    .map(|m| m.id)
                    .filter(|id| seen_ids.insert(id.clone())),
            );
            match page.next_page_token.filter(|t| !t.is_empty()) {
                Some(next) if seen_tokens.insert(next.clone()) => page_token = Some(next),
                _ => break,
            }
        }
        let mut messages = Vec::with_capacity(ids.len());
        for id in &ids {
            messages.push(self.metadata(id)?);
        }
        messages.sort_by(|a, b| (a.internal_date_ms, &a.id).cmp(&(b.internal_date_ms, &b.id)));
        Ok(messages)
    }

    fn metadata(&mut self, id: &str) -> Result<Message, String> {
        let url = self.url(&["messages", id])?;
        let query = [
            ("format", "metadata"),
            ("metadataHeaders", "From"),
            ("metadataHeaders", "Subject"),
        ];
        let meta: Metadata = self
            .get(url, &query)?
            .json()
            .map_err(|_| t(msg::RESPONSE_UNREADABLE))?;
        let internal_date_ms = meta
            .internal_date
            .parse()
            .map_err(|_| t(msg::RESPONSE_UNREADABLE))?;
        let header = |name: &str| {
            meta.payload
                .headers
                .iter()
                .find(|h| h.name.eq_ignore_ascii_case(name))
                .map(|h| h.value.clone())
                .unwrap_or_default()
        };
        Ok(Message {
            from: header("From"),
            subject: header("Subject"),
            thread_id: if meta.thread_id.is_empty() {
                meta.id.clone()
            } else {
                meta.thread_id
            },
            id: meta.id,
            internal_date_ms,
        })
    }
}

pub(crate) fn http() -> Result<Http, String> {
    Http::builder()
        .user_agent(concat!("focusdeck-gmail/", env!("CARGO_PKG_VERSION")))
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|_| t(msg::HTTP_CLIENT_FAILED))
}

pub(crate) fn transport_error(e: reqwest::Error) -> String {
    if e.is_timeout() {
        t(msg::TIMED_OUT)
    } else if e.is_connect() {
        t(msg::CONNECT_FAILED)
    } else {
        t(msg::REQUEST_FAILED)
    }
}

pub(crate) fn status_error(code: u16) -> String {
    t_args(msg::HTTP_STATUS, &[("status", i64::from(code).into())])
}
