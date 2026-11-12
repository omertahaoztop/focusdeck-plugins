use chrono::{DateTime, Datelike, FixedOffset, Utc};
use focusdeck_protocol::{
    ActionParams, Describe, DropParams, Item, ListParams, PROTOCOL_VERSION, Plugin, Reminder,
    RemindersParams, Source,
};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::client::{Client, Label, Task};
use crate::i18n::{msg, t, t_args};

const SOURCE_MARKER: &str = "focusdeck-source: ";
const FROM_LABEL_PREFIX: &str = "kimden:";
const POSITION_STEP: f64 = 65536.0;

#[derive(Deserialize)]
struct Config {
    url: String,
    token_file: String,
    project: String,
    #[serde(default)]
    bucket: Option<String>,
}

struct Target {
    client: Client,
    base: String,
    project: u64,
    view: u64,
    default_bucket: Option<u64>,
    done_bucket: Option<u64>,
    bucket: Option<u64>,
}

struct Entry {
    task: Task,
    bucket_title: String,
}

#[derive(Default)]
pub struct Vikunja {
    target: Option<Target>,
    last: Vec<(u64, f64)>,
}

impl Target {
    fn open_tasks(&self) -> Result<Vec<Entry>, String> {
        let mut entries = Vec::new();
        for bucket in self.client.view_tasks(self.project, self.view)? {
            let hidden = Some(bucket.id) == self.done_bucket
                || self.bucket.is_some_and(|only| only != bucket.id);
            if hidden {
                continue;
            }
            for task in bucket.tasks.into_iter().filter(|t| !t.done) {
                entries.push(Entry {
                    task,
                    bucket_title: bucket.title.clone(),
                });
            }
        }
        entries.sort_by(|a, b| a.task.position.total_cmp(&b.task.position));
        Ok(entries)
    }

    fn item(&self, entry: &Entry) -> Item {
        let column = self.bucket.is_none().then_some(entry.bucket_title.as_str());
        to_item(&entry.task, &self.base, column)
    }

    fn move_to(&self, task: u64, bucket: u64) -> Result<(), String> {
        self.client
            .move_to_bucket(self.project, self.view, bucket, task)
    }

    fn bucket_by_title(&self, title: &str) -> Result<u64, String> {
        find_bucket(&self.client, self.project, self.view, title)
    }

    fn label_id(&self, title: &str) -> Result<u64, String> {
        match self.client.find_label(title)? {
            Some(label) => Ok(label.id),
            None => Ok(self.client.create_label(title)?.id),
        }
    }

    fn create(&self, item: &Item, source: Option<&Source>) -> Result<Item, String> {
        let mut titles = item.labels.clone();
        if let Some(from) = item.from.as_deref().filter(|f| !f.is_empty()) {
            titles.push(format!("{FROM_LABEL_PREFIX}{from}"));
        }
        let mut labels = Vec::new();
        for title in titles {
            if labels.iter().any(|l: &Label| l.title == title) {
                continue;
            }
            let id = self.label_id(&title)?;
            labels.push(Label { id, title });
        }
        let mut body = json!({
            "title": item.title,
            "description": description_html(item, source),
        });
        if let Some(due) = item.due {
            body["due_date"] = json!(due.to_rfc3339());
            body["reminders"] = json!([{"reminder": due.to_rfc3339()}]);
        }
        let mut task = self.client.create_task(self.project, &body)?;
        if !labels.is_empty() {
            let ids: Vec<u64> = labels.iter().map(|l| l.id).collect();
            self.client.set_labels(task.id, &ids)?;
        }
        task.labels = labels;
        if let Some(bucket) = self.bucket {
            self.move_to(task.id, bucket)?;
        }
        Ok(to_item(&task, &self.base, None))
    }

    fn adopt(&self, id: u64) -> Result<Item, String> {
        let mut raw = self.client.get_task_raw(id)?;
        let task: Task =
            serde_json::from_value(raw.clone()).map_err(|_| t(msg::RESPONSE_UNREADABLE))?;
        let bucket = self.bucket.or(self.default_bucket);
        if task.project_id == self.project {
            let bucket = bucket.ok_or_else(|| t(msg::NO_DEFAULT_BUCKET))?;
            self.move_to(id, bucket)?;
            return Ok(to_item(&task, &self.base, None));
        }
        raw.as_object_mut()
            .ok_or_else(|| t(msg::RESPONSE_UNREADABLE))?
            .insert("project_id".into(), json!(self.project));
        let moved = self.client.update_task(id, &raw)?;
        if let Some(bucket) = self.bucket {
            self.move_to(id, bucket)?;
        }
        Ok(to_item(&moved, &self.base, None))
    }
}

impl Vikunja {
    fn target(&self) -> Result<&Target, String> {
        self.target.as_ref().ok_or_else(|| t(msg::NOT_CONFIGURED))
    }

    fn refresh(&mut self) -> Result<Vec<Item>, String> {
        let target = self.target()?;
        let entries = target.open_tasks()?;
        let items = entries.iter().map(|e| target.item(e)).collect();
        self.last = entries
            .iter()
            .map(|e| (e.task.id, e.task.position))
            .collect();
        Ok(items)
    }

    fn reorder(&mut self, id: u64, args: &Value) -> Result<(), String> {
        let wanted = args
            .get("position")
            .and_then(Value::as_u64)
            .ok_or_else(|| t(msg::POSITION_REQUIRED))?;
        if !self.last.iter().any(|(known, _)| *known == id) {
            self.refresh()?;
        }
        if !self.last.iter().any(|(known, _)| *known == id) {
            return Err(t_args(
                msg::TASK_NOT_LISTED,
                &[("id", id.to_string().as_str().into())],
            ));
        }
        let others: Vec<f64> = self
            .last
            .iter()
            .filter(|(known, _)| *known != id)
            .map(|(_, position)| *position)
            .collect();
        let position = place_between(&others, usize::try_from(wanted).unwrap_or(usize::MAX));
        let target = self.target()?;
        target.client.set_position(target.view, id, position)?;
        if let Some(entry) = self.last.iter_mut().find(|(known, _)| *known == id) {
            entry.1 = position;
        }
        self.last.sort_by(|a, b| a.1.total_cmp(&b.1));
        Ok(())
    }
}

impl Plugin for Vikunja {
    fn describe(&self) -> Describe {
        Describe {
            name: "vikunja".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            protocol: PROTOCOL_VERSION,
            actions: ["done", "delete", "move", "reorder"]
                .map(String::from)
                .to_vec(),
            accepts_drop: true,
            reminders: true,
        }
    }

    fn configure(&mut self, config: Value) -> Result<(), String> {
        let cfg: Config = serde_json::from_value(config).map_err(|e| {
            t_args(
                msg::INVALID_SETTINGS,
                &[("error", e.to_string().as_str().into())],
            )
        })?;
        let token = std::fs::read_to_string(&cfg.token_file).map_err(|_| {
            t_args(
                msg::TOKEN_UNREADABLE,
                &[("path", cfg.token_file.as_str().into())],
            )
        })?;
        let token = token.trim();
        if token.is_empty() {
            return Err(t_args(
                msg::TOKEN_EMPTY,
                &[("path", cfg.token_file.as_str().into())],
            ));
        }
        let client = Client::new(&cfg.url, token)?;
        let wanted = cfg.project.to_lowercase();
        let project = client
            .projects()?
            .into_iter()
            .find(|p| p.title.to_lowercase() == wanted)
            .ok_or_else(|| {
                t_args(
                    msg::PROJECT_MISSING,
                    &[("project", cfg.project.as_str().into())],
                )
            })?;
        let view = client
            .views(project.id)?
            .into_iter()
            .find(|v| v.kanban)
            .ok_or_else(|| {
                t_args(
                    msg::NO_KANBAN_VIEW,
                    &[("project", cfg.project.as_str().into())],
                )
            })?;
        let bucket = match cfg.bucket.as_deref() {
            Some(title) => Some(find_bucket(&client, project.id, view.id, title)?),
            None => None,
        };
        self.target = Some(Target {
            client,
            base: cfg.url.trim_end_matches('/').to_string(),
            project: project.id,
            view: view.id,
            default_bucket: Some(view.default_bucket_id).filter(|id| *id != 0),
            done_bucket: Some(view.done_bucket_id).filter(|id| *id != 0),
            bucket,
        });
        self.last.clear();
        Ok(())
    }

    fn list(&mut self, _p: ListParams) -> Result<Vec<Item>, String> {
        self.refresh()
    }

    fn action(&mut self, p: ActionParams) -> Result<Option<Item>, String> {
        let id = parse_id(&p.item)?;
        match p.action.as_str() {
            "done" => {
                let target = self.target()?;
                let done = target.done_bucket.ok_or_else(|| t(msg::NO_DONE_BUCKET))?;
                target.move_to(id, done)?;
            }
            "delete" => self.target()?.client.delete_task(id)?,
            "reorder" => self.reorder(id, &p.args)?,
            "move" => match p.args.get("bucket").and_then(Value::as_str) {
                Some(title) => {
                    let target = self.target()?;
                    target.move_to(id, target.bucket_by_title(title)?)?;
                }
                None => self.reorder(id, &p.args)?,
            },
            other => return Err(t_args(msg::UNKNOWN_ACTION, &[("action", other.into())])),
        }
        Ok(None)
    }

    fn drop_item(&mut self, p: DropParams) -> Result<Item, String> {
        let target = self.target()?;
        if let Some(origin) = p.from.as_ref().filter(|s| s.plugin == "vikunja") {
            return target.adopt(parse_id(&origin.id)?);
        }
        let source = p.item.source.as_ref().or(p.from.as_ref());
        target.create(&p.item, source)
    }

    fn reminders(&mut self, p: RemindersParams) -> Result<Vec<Reminder>, String> {
        let target = self.target()?;
        let mut found = Vec::new();
        for entry in target.open_tasks()? {
            for reminder in &entry.task.reminders {
                if let Some(at) = parse_time(&reminder.reminder).filter(|at| *at > p.since) {
                    found.push(Reminder {
                        at,
                        item: target.item(&entry),
                    });
                }
            }
        }
        found.sort_by_key(|r| r.at);
        Ok(found)
    }
}

fn find_bucket(client: &Client, project: u64, view: u64, title: &str) -> Result<u64, String> {
    let wanted = title.to_lowercase();
    client
        .buckets(project, view)?
        .into_iter()
        .find(|b| b.title.to_lowercase() == wanted)
        .map(|b| b.id)
        .ok_or_else(|| t_args(msg::BUCKET_MISSING, &[("title", title.into())]))
}

fn parse_id(raw: &str) -> Result<u64, String> {
    raw.parse()
        .map_err(|_| t_args(msg::INVALID_TASK_ID, &[("id", raw.into())]))
}

fn place_between(others: &[f64], wanted: usize) -> f64 {
    match (others.first(), others.last()) {
        (None, _) | (_, None) => POSITION_STEP,
        (Some(first), _) if wanted == 0 => {
            if *first > 0.0 {
                first / 2.0
            } else {
                first - POSITION_STEP
            }
        }
        (_, Some(last)) if wanted >= others.len() => last + POSITION_STEP,
        _ => (others[wanted - 1] + others[wanted]) / 2.0,
    }
}

fn parse_time(raw: &str) -> Option<DateTime<FixedOffset>> {
    let time = DateTime::parse_from_rfc3339(raw).ok()?;
    (time.year() > 1).then_some(time)
}

fn parse_source(description: &str) -> Option<Source> {
    let start = description.find(SOURCE_MARKER)? + SOURCE_MARKER.len();
    let token = description[start..]
        .split(|c: char| c.is_whitespace() || c == '<')
        .next()?;
    let (plugin, id) = token.split_once(':')?;
    (!plugin.is_empty() && !id.is_empty()).then(|| Source {
        plugin: plugin.into(),
        id: id.into(),
    })
}

fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

fn description_html(item: &Item, source: Option<&Source>) -> String {
    let mut html = String::new();
    if let Some(text) = item
        .description
        .as_deref()
        .map(str::trim)
        .filter(|t| !t.is_empty())
    {
        let text = escape(&text.replace("\r\n", "\n")).replace('\n', "<br>");
        html.push_str(&format!("<p>{text}</p>"));
    }
    if !item.links.is_empty() {
        let links: Vec<String> = item
            .links
            .iter()
            .map(|l| format!("<a href=\"{}\">{}</a>", escape(&l.url), escape(&l.label)))
            .collect();
        html.push_str(&format!("<p>{}</p>", links.join(" ")));
    }
    if let Some(source) = source {
        html.push_str(&format!(
            "<p>{SOURCE_MARKER}{}</p>",
            escape(&format!("{}:{}", source.plugin, source.id))
        ));
    }
    html
}

fn to_item(task: &Task, base: &str, column: Option<&str>) -> Item {
    let due = task.due_date.as_deref().and_then(parse_time);
    let overdue = due.is_some_and(|d| d < Utc::now());
    let from = task
        .labels
        .iter()
        .find_map(|l| l.title.strip_prefix(FROM_LABEL_PREFIX))
        .map(str::to_string);
    let labels = task
        .labels
        .iter()
        .filter(|l| !l.title.starts_with(FROM_LABEL_PREFIX))
        .map(|l| l.title.clone())
        .collect();
    let mut parts = Vec::new();
    if let Some(from) = &from {
        parts.push(format!("@{from}"));
    }
    parts.extend(column.filter(|c| !c.is_empty()).map(str::to_string));
    Item {
        id: task.id.to_string(),
        title: task.title.clone(),
        subtitle: (!parts.is_empty()).then(|| parts.join(" · ")),
        url: Some(format!("{base}/tasks/{}", task.id)),
        due,
        overdue,
        source: parse_source(&task.description),
        description: None,
        labels,
        from,
        links: vec![],
    }
}
