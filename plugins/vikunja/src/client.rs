use std::time::Duration;

use reqwest::StatusCode;
use reqwest::blocking::{Client as Http, RequestBuilder};
use reqwest::header::{AUTHORIZATION, HeaderMap, HeaderValue};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer};
use serde_json::{Value, json};

const KANBAN_KIND: u64 = 3;

fn null_as_default<'de, D, T>(d: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de> + Default,
{
    Ok(Option::<T>::deserialize(d)?.unwrap_or_default())
}

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
}

pub struct Client {
    http: Http,
    api: String,
}

impl Client {
    pub fn new(base_url: &str, token: &str) -> Result<Self, String> {
        let mut auth = HeaderValue::from_str(&format!("Bearer {token}"))
            .map_err(|_| "Token geçersiz karakter içeriyor".to_string())?;
        auth.set_sensitive(true);
        let mut headers = HeaderMap::new();
        headers.insert(AUTHORIZATION, auth);
        let http = Http::builder()
            .default_headers(headers)
            .user_agent(concat!("focusdeck-vikunja/", env!("CARGO_PKG_VERSION")))
            .timeout(Duration::from_secs(10))
            .build()
            .map_err(|_| "HTTP istemcisi kurulamadı".to_string())?;
        Ok(Self {
            http,
            api: format!("{}/api/v1", base_url.trim_end_matches('/')),
        })
    }

    fn send(&self, request: RequestBuilder) -> Result<Value, String> {
        let response = request.send().map_err(transport_error)?;
        let status = response.status();
        if !status.is_success() {
            return Err(status_error(status));
        }
        let body = response.text().map_err(transport_error)?;
        if body.trim().is_empty() {
            return Ok(Value::Null);
        }
        serde_json::from_str(&body).map_err(|_| "Vikunja: yanıt çözülemedi".to_string())
    }

    fn typed<T: DeserializeOwned>(value: Value) -> Result<T, String> {
        serde_json::from_value(value).map_err(|_| "Vikunja: yanıt çözülemedi".to_string())
    }

    fn get<T: DeserializeOwned>(&self, path: &str, query: &[(&str, &str)]) -> Result<T, String> {
        let url = format!("{}{path}", self.api);
        Self::typed(self.send(self.http.get(url).query(query))?)
    }

    fn put<T: DeserializeOwned>(&self, path: &str, body: &Value) -> Result<T, String> {
        let url = format!("{}{path}", self.api);
        Self::typed(self.send(self.http.put(url).json(body))?)
    }

    fn post<T: DeserializeOwned>(&self, path: &str, body: &Value) -> Result<T, String> {
        let url = format!("{}{path}", self.api);
        Self::typed(self.send(self.http.post(url).json(body))?)
    }

    pub fn projects(&self) -> Result<Vec<Project>, String> {
        self.get("/projects", &[("per_page", "200")])
    }

    pub fn views(&self, project: u64) -> Result<Vec<ProjectView>, String> {
        self.get(&format!("/projects/{project}/views"), &[])
    }

    pub fn buckets(&self, project: u64, view: u64) -> Result<Vec<Bucket>, String> {
        self.get(&format!("/projects/{project}/views/{view}/buckets"), &[])
    }

    pub fn view_tasks(&self, project: u64, view: u64) -> Result<Vec<Bucket>, String> {
        self.get(
            &format!("/projects/{project}/views/{view}/tasks"),
            &[("per_page", "200")],
        )
    }

    pub fn get_task_raw(&self, id: u64) -> Result<Value, String> {
        self.get(&format!("/tasks/{id}"), &[])
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

fn transport_error(e: reqwest::Error) -> String {
    if e.is_timeout() {
        "Vikunja: zaman aşımı".into()
    } else if e.is_connect() {
        "Vikunja: bağlantı kurulamadı".into()
    } else {
        "Vikunja: istek başarısız".into()
    }
}

fn status_error(status: StatusCode) -> String {
    let reason = match status.as_u16() {
        400 => "geçersiz istek",
        401 => "token geçersiz",
        403 => "yetki yok",
        404 => "bulunamadı",
        429 => "çok fazla istek",
        500..=599 => "sunucu hatası",
        _ => "beklenmeyen yanıt",
    };
    format!("Vikunja {}: {reason}", status.as_u16())
}
