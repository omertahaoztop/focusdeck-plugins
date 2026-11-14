use std::collections::HashSet;
use std::time::Duration;

use reqwest::StatusCode;
use reqwest::blocking::{Client as Http, RequestBuilder, Response};
use reqwest::header::{AUTHORIZATION, HeaderMap, HeaderValue};
use seenboard_plugin_common::{StatusReason, Transport, http_client, null_as_default};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer};
use serde_json::{Value, json};

use crate::i18n::{msg, t, t_args};

const KANBAN_KIND: u64 = 3;
const PER_PAGE: usize = 200;
const MAX_PAGES: usize = 1000;

fn kind_is_kanban<'de, D: Deserializer<'de>>(d: D) -> Result<bool, D::Error> {
    Ok(match Value::deserialize(d)? {
        Value::String(kind) => kind.eq_ignore_ascii_case("kanban"),
        Value::Number(kind) => kind.as_u64() == Some(KANBAN_KIND),
        _ => false,
    })
}

#[derive(Debug, Clone, Deserialize)]
pub struct Project {
    pub id: u64,
    pub title: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ProjectView {
    pub id: u64,
    #[serde(default, rename = "view_kind", deserialize_with = "kind_is_kanban")]
    pub kanban: bool,
    #[serde(default)]
    pub default_bucket_id: u64,
    #[serde(default)]
    pub done_bucket_id: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Label {
    pub id: u64,
    pub title: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Reminder {
    #[serde(default, deserialize_with = "null_as_default")]
    pub reminder: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Task {
    pub id: u64,
    #[serde(default, deserialize_with = "null_as_default")]
    pub title: String,
    #[serde(default, deserialize_with = "null_as_default")]
    pub description: String,
    #[serde(default)]
    pub done: bool,
    #[serde(default)]
    pub due_date: Option<String>,
    #[serde(default)]
    pub position: f64,
    #[serde(default)]
    pub bucket_id: u64,
    #[serde(default)]
    pub project_id: u64,
    #[serde(default, deserialize_with = "null_as_default")]
    pub reminders: Vec<Reminder>,
    #[serde(default, deserialize_with = "null_as_default")]
    pub labels: Vec<Label>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Bucket {
    pub id: u64,
    #[serde(default, deserialize_with = "null_as_default")]
    pub title: String,
    #[serde(default, deserialize_with = "null_as_default")]
    pub tasks: Vec<Task>,
    #[serde(default)]
    pub count: u64,
}

pub struct Client {
    http: Http,
    api: String,
}

impl Client {
    pub fn new(base: &str, token: &str) -> Result<Self, String> {
        let mut auth = HeaderValue::from_str(&format!("Bearer {token}"))
            .map_err(|_| t(msg::TOKEN_INVALID_CHARS))?;
        auth.set_sensitive(true);
        let mut headers = HeaderMap::new();
        headers.insert(AUTHORIZATION, auth);
        let http = http_client(
            concat!("seenboard-vikunja/", env!("CARGO_PKG_VERSION")),
            Duration::from_secs(10),
            &[base],
        )
        .default_headers(headers)
        .build()
        .map_err(|_| t(msg::HTTP_CLIENT_FAILED))?;
        Ok(Self {
            http,
            api: format!("{base}/api/v1"),
        })
    }

    fn dispatch(request: RequestBuilder) -> Result<Response, String> {
        request.send().map_err(|e| transport_error(&e))
    }

    fn body(response: Response) -> Result<Value, String> {
        let status = response.status();
        if !status.is_success() {
            return Err(status_error(status.as_u16()));
        }
        let body = response.text().map_err(|e| transport_error(&e))?;
        if body.trim().is_empty() {
            return Ok(Value::Null);
        }
        serde_json::from_str(&body).map_err(|_| t(msg::RESPONSE_UNREADABLE))
    }

    fn send(&self, request: RequestBuilder) -> Result<Value, String> {
        Self::body(Self::dispatch(request)?)
    }

    fn typed<T: DeserializeOwned>(value: Value) -> Result<T, String> {
        serde_json::from_value(value).map_err(|_| t(msg::RESPONSE_UNREADABLE))
    }

    fn get<T: DeserializeOwned>(&self, path: &str, query: &[(&str, &str)]) -> Result<T, String> {
        let url = format!("{}{path}", self.api);
        Self::typed(self.send(self.http.get(url).query(query))?)
    }

    fn get_page<T: DeserializeOwned>(&self, path: &str, page: usize) -> Result<T, String> {
        let page = page.to_string();
        let per_page = PER_PAGE.to_string();
        self.get(path, &[("per_page", &per_page), ("page", &page)])
    }

    fn put<T: DeserializeOwned>(&self, path: &str, body: &Value) -> Result<T, String> {
        let url = format!("{}{path}", self.api);
        Self::typed(self.send(self.http.put(url).json(body))?)
    }

    fn post<T: DeserializeOwned>(&self, path: &str, body: &Value) -> Result<T, String> {
        let url = format!("{}{path}", self.api);
        Self::typed(self.send(self.http.post(url).json(body))?)
    }

    // the server may cap per_page below ours, so only an empty or repeated page ends the listing
    pub fn projects(&self) -> Result<Vec<Project>, String> {
        let mut all: Vec<Project> = Vec::new();
        let mut seen = HashSet::new();
        for page in 1..=MAX_PAGES {
            let batch: Vec<Project> = self.get_page("/projects", page)?;
            let known = all.len();
            all.extend(batch.into_iter().filter(|p| seen.insert(p.id)));
            if all.len() == known {
                break;
            }
        }
        Ok(all)
    }

    pub fn views(&self, project: u64) -> Result<Vec<ProjectView>, String> {
        self.get(&format!("/projects/{project}/views"), &[])
    }

    pub fn buckets(&self, project: u64, view: u64) -> Result<Vec<Bucket>, String> {
        self.get(&format!("/projects/{project}/views/{view}/buckets"), &[])
    }

    /// Kanban pages are per bucket: later pages are merged into the buckets of the first one.
    pub fn view_tasks(&self, project: u64, view: u64) -> Result<Vec<Bucket>, String> {
        let path = format!("/projects/{project}/views/{view}/tasks");
        let mut merged: Vec<Bucket> = Vec::new();
        let mut seen: Vec<HashSet<u64>> = Vec::new();
        for page in 1..=MAX_PAGES {
            let batch: Vec<Bucket> = self.get_page(&path, page)?;
            let mut grew = false;
            let mut more = false;
            for bucket in batch {
                let at = match merged.iter().position(|b| b.id == bucket.id) {
                    Some(at) => at,
                    None => {
                        merged.push(Bucket {
                            id: bucket.id,
                            title: bucket.title,
                            tasks: Vec::new(),
                            count: bucket.count,
                        });
                        seen.push(HashSet::new());
                        merged.len() - 1
                    }
                };
                let have = &mut merged[at];
                let known = &mut seen[at];
                let returned = bucket.tasks.len();
                let before = have.tasks.len();
                have.tasks
                    .extend(bucket.tasks.into_iter().filter(|t| known.insert(t.id)));
                grew |= have.tasks.len() > before;
                let total = usize::try_from(bucket.count).unwrap_or(usize::MAX);
                more |= if total > 0 {
                    total > have.tasks.len()
                } else {
                    returned >= PER_PAGE
                };
            }
            if !(grew && more) {
                break;
            }
        }
        Ok(merged)
    }

    pub fn get_task_raw(&self, id: u64) -> Result<Value, String> {
        self.get(&format!("/tasks/{id}"), &[])
    }

    /// A task that is missing or not ours to read is reported as absent rather than as a failure.
    pub fn find_task_raw(&self, id: u64) -> Result<Option<Value>, String> {
        let url = format!("{}/tasks/{id}", self.api);
        let response = Self::dispatch(self.http.get(url))?;
        if matches!(
            response.status(),
            StatusCode::FORBIDDEN | StatusCode::NOT_FOUND
        ) {
            return Ok(None);
        }
        Self::body(response).map(Some)
    }

    pub fn create_task(&self, project: u64, body: &Value) -> Result<Task, String> {
        self.put(&format!("/projects/{project}/tasks"), body)
    }

    /// Vikunja resets every field missing from the body, so callers pass the full task object.
    pub fn update_task(&self, id: u64, full_task: &Value) -> Result<Task, String> {
        self.post(&format!("/tasks/{id}"), full_task)
    }

    pub fn delete_task(&self, id: u64) -> Result<(), String> {
        let url = format!("{}/tasks/{id}", self.api);
        self.send(self.http.delete(url)).map(drop)
    }

    pub fn move_to_bucket(
        &self,
        project: u64,
        view: u64,
        bucket: u64,
        task: u64,
    ) -> Result<(), String> {
        let body = json!({"task_id": task, "bucket_id": bucket, "project_view_id": view});
        self.post::<Value>(
            &format!("/projects/{project}/views/{view}/buckets/{bucket}/tasks"),
            &body,
        )
        .map(drop)
    }

    pub fn set_position(&self, view: u64, task: u64, position: f64) -> Result<(), String> {
        let body = json!({"task_id": task, "project_view_id": view, "position": position});
        self.post::<Value>(&format!("/tasks/{task}/position"), &body)
            .map(drop)
    }

    pub fn find_label(&self, title: &str) -> Result<Option<Label>, String> {
        let found: Vec<Label> = self.get("/labels", &[("s", title)])?;
        Ok(found
            .into_iter()
            .find(|l| l.title.eq_ignore_ascii_case(title)))
    }

    pub fn create_label(&self, title: &str) -> Result<Label, String> {
        self.put("/labels", &json!({"title": title}))
    }

    pub fn set_labels(&self, task: u64, labels: &[u64]) -> Result<(), String> {
        let labels: Vec<Value> = labels.iter().map(|id| json!({"id": id})).collect();
        self.post::<Value>(
            &format!("/tasks/{task}/labels/bulk"),
            &json!({"labels": labels}),
        )
        .map(drop)
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
