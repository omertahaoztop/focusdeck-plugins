use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::time::{Duration, Instant};

use focusdeck_plugin_common::{StatusReason, Transport, UrlError, http_client};
use reqwest::blocking::{Client as Http, Response};
use reqwest::{StatusCode, Url};
use serde::Deserialize;

use crate::i18n::{msg, t, t_args};
use crate::oauth::{ClientSecret, TokenError, request_token};

const RENEW_MARGIN: Duration = Duration::from_secs(60);
const MAX_LIFETIME_SECS: u64 = 24 * 60 * 60;
const MAX_PAGES: usize = 100;

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
    api: Url,
    secret: ClientSecret,
    refresh_token: String,
    access: Option<Access>,
    cache: HashMap<String, Message>,
}

impl Client {
    pub fn new(api: Url, secret: ClientSecret, refresh_token: String) -> Result<Self, String> {
        Ok(Self {
            http: http(&[&api, &secret.token_uri])?,
            api,
            secret,
            refresh_token,
            access: None,
            cache: HashMap::new(),
        })
    }

    pub fn authorize(&mut self) -> Result<(), String> {
        let grant = [
            ("grant_type", "refresh_token"),
            ("refresh_token", self.refresh_token.as_str()),
        ];
        let response = match request_token(&self.http, &self.secret, &grant) {
            Ok(response) => response,
            Err(TokenError::Status(400 | 401)) => return Err(session_lost()),
            Err(TokenError::Status(code)) => return Err(status_error(code)),
            Err(TokenError::Other(message)) => return Err(message),
        };
        if response.access_token.is_empty() {
            return Err(t(msg::RESPONSE_UNREADABLE));
        }
        let lifetime = Duration::from_secs(response.expires_in.min(MAX_LIFETIME_SECS));
        self.access = Some(Access {
            token: response.access_token,
            expires_at: Instant::now() + lifetime,
        });
        Ok(())
    }

    fn send(&mut self, url: &Url, query: &[(&str, &str)]) -> Result<Response, String> {
        let fresh = self
            .access
            .as_ref()
            .is_some_and(|a| Instant::now() + RENEW_MARGIN < a.expires_at);
        if !fresh {
            self.authorize()?;
        }
        let access = self
            .access
            .as_ref()
            .ok_or_else(|| t(msg::RESPONSE_UNREADABLE))?;
        self.http
            .get(url.clone())
            .query(query)
            .bearer_auth(&access.token)
            .send()
            .map_err(|e| transport_error(&e))
    }

    // Google can revoke an access token before its stated expiry, so one 401 earns a renewal
    fn get(&mut self, url: &Url, query: &[(&str, &str)]) -> Result<Response, String> {
        let mut response = self.send(url, query)?;
        if response.status() == StatusCode::UNAUTHORIZED {
            self.access = None;
            response = self.send(url, query)?;
        }
        match response.status() {
            status if status.is_success() => Ok(response),
            status => Err(status_error(status.as_u16())),
        }
    }

    fn url(&self, segments: &[&str]) -> Result<Url, String> {
        let mut url = self.api.clone();
        url.path_segments_mut()
            .map_err(|()| url_error("api_url", UrlError::Invalid))?
            .pop_if_empty()
            .extend(segments);
        Ok(url)
    }

    pub fn starred(&mut self) -> Result<Vec<&Message>, String> {
        let list_url = self.url(&["messages"])?;
        let mut ids = HashSet::new();
        let mut seen_tokens = HashSet::new();
        let mut page_token: Option<String> = None;
        for _ in 0..MAX_PAGES {
            let mut query = vec![("q", "is:starred"), ("maxResults", "100")];
            if let Some(token) = &page_token {
                query.push(("pageToken", token));
            }
            let page: IdPage = self
                .get(&list_url, &query)?
                .json()
                .map_err(|_| t(msg::RESPONSE_UNREADABLE))?;
            ids.extend(page.messages.into_iter().map(|m| m.id));
            match page.next_page_token.filter(|t| !t.is_empty()) {
                Some(next) if seen_tokens.insert(next.clone()) => page_token = Some(next),
                _ => break,
            }
        }
        self.cache.retain(|id, _| ids.contains(id));
        for id in ids {
            if !self.cache.contains_key(&id) {
                let message = self.metadata(&id)?;
                self.cache.insert(id, message);
            }
        }
        let mut messages: Vec<&Message> = self.cache.values().collect();
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
            .get(&url, &query)?
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

pub(crate) fn http(targets: &[&Url]) -> Result<Http, String> {
    let targets: Vec<&str> = targets.iter().map(|url| url.as_str()).collect();
    http_client(
        concat!("focusdeck-gmail/", env!("CARGO_PKG_VERSION")),
        Duration::from_secs(15),
        &targets,
    )
    .build()
    .map_err(|_| t(msg::HTTP_CLIENT_FAILED))
}

fn session_lost() -> String {
    let bin = std::env::current_exe().ok();
    let bin = bin
        .as_deref()
        .and_then(Path::to_str)
        .unwrap_or("focusdeck-gmail");
    t_args(msg::SESSION_LOST, &[("bin", bin.into())])
}

pub(crate) fn url_error(field: &str, error: UrlError) -> String {
    let id = match error {
        UrlError::Invalid => msg::URL_INVALID,
        UrlError::Insecure => msg::URL_INSECURE,
    };
    t_args(id, &[("field", field.into())])
}

pub(crate) fn transport_error(error: &reqwest::Error) -> String {
    t(match Transport::of(error) {
        Transport::TimedOut => msg::TIMED_OUT,
        Transport::Connect => msg::CONNECT_FAILED,
        Transport::Other => msg::REQUEST_FAILED,
    })
}

pub(crate) fn status_error(status: u16) -> String {
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
