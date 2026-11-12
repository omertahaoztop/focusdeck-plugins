use std::time::{Duration, Instant};

use chrono::{DateTime, Datelike, FixedOffset, Utc};
use focusdeck_plugin_common::{SecretError, UrlError, checked_base, printable, read_secret};
use focusdeck_protocol::{
    ActionParams, Describe, DropParams, Item, ListParams, PROTOCOL_VERSION, Plugin, Reminder,
    RemindersParams, Source,
};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::client::{Client, Label, Task};
use crate::i18n::{msg, t, t_args};

const SOURCE_MARKER: &str = "focusdeck-source: ";
const DEFAULT_FROM_LABEL_PREFIX: &str = "kimden:";
const LINK_SCHEMES: [&str; 3] = ["https://", "http://", "mailto:"];
const POSITION_STEP: f64 = 65536.0;
const SNAPSHOT_TTL: Duration = Duration::from_secs(30);

#[derive(Deserialize)]
struct Config {
    url: String,
    token_file: String,
    project: String,
    #[serde(default)]
    bucket: Option<String>,
    #[serde(default)]
    from_label_prefix: Option<String>,
}

struct Target {
    client: Client,
    base: String,
    from_prefix: String,
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

struct Snapshot {
    entries: Vec<Entry>,
    at: Instant,
}

#[derive(Default)]
pub struct Vikunja {
    target: Option<Target>,
    last: Option<Snapshot>,
}

impl Target {
    fn snapshot(&self) -> Result<Snapshot, String> {
        let mut entries = Vec::new();
        for bucket in self.client.view_tasks(self.project, self.view)? {
            let hidden = Some(bucket.id) == self.done_bucket
                || self.bucket.is_some_and(|only| only != bucket.id);
            if hidden {
                continue;
            }
            let title = printable(&bucket.title);
            for task in bucket.tasks.into_iter().filter(|t| !t.done) {
                entries.push(Entry {
                    task,
                    bucket_title: title.clone(),
                });
            }
        }
        entries.sort_by(|a, b| a.task.position.total_cmp(&b.task.position));
        Ok(Snapshot {
            entries,
            at: Instant::now(),
        })
    }

    fn item(&self, entry: &Entry, now: DateTime<Utc>) -> Item {
        let column = self.bucket.is_none().then_some(entry.bucket_title.as_str());
        to_item(&entry.task, &self.base, &self.from_prefix, column, now)
    }

    fn bare_item(&self, task: &Task) -> Item {
        to_item(task, &self.base, &self.from_prefix, None, Utc::now())
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
        if !self.from_prefix.is_empty()
            && let Some(from) = item.from.as_deref().filter(|f| !f.is_empty())
        {
            titles.push(format!("{}{from}", self.from_prefix));
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
        if let Err(error) = self.place(task.id, &labels) {
            // a half-built task would sit beside the one the retry creates
            return Err(match self.client.delete_task(task.id) {
                Ok(()) => error,
                Err(_) => t_args(
                    msg::TASK_LEFT_BEHIND,
                    &[
                        ("error", error.as_str().into()),
                        ("id", task.id.to_string().as_str().into()),
                    ],
                ),
            });
        }
        task.labels = labels;
        Ok(self.bare_item(&task))
    }

    fn place(&self, task: u64, labels: &[Label]) -> Result<(), String> {
        if !labels.is_empty() {
            let ids: Vec<u64> = labels.iter().map(|l| l.id).collect();
            self.client.set_labels(task, &ids)?;
        }
        match self.bucket {
            Some(bucket) => self.move_to(task, bucket),
            None => Ok(()),
        }
    }

    // the origin only says "vikunja", so the id may belong to another server; the item's link names its own
    fn adopt(&self, origin_id: &str, url: Option<&str>) -> Result<Option<Item>, String> {
        let Ok(id) = origin_id.parse::<u64>() else {
            return Ok(None);
        };
        if url != Some(format!("{}/tasks/{id}", self.base).as_str()) {
            return Ok(None);
        }
        let Some(mut raw) = self.client.find_task_raw(id)? else {
            return Ok(None);
        };
        let task = Task::deserialize(&raw).map_err(|_| t(msg::RESPONSE_UNREADABLE))?;
        let bucket = self.bucket.or(self.default_bucket);
        if task.project_id == self.project {
            let bucket = bucket.ok_or_else(|| t(msg::NO_DEFAULT_BUCKET))?;
            self.move_to(id, bucket)?;
            return Ok(Some(self.bare_item(&task)));
        }
        raw.as_object_mut()
            .ok_or_else(|| t(msg::RESPONSE_UNREADABLE))?
            .insert("project_id".into(), json!(self.project));
        let moved = self.client.update_task(id, &raw)?;
        if let Some(bucket) = self.bucket {
            self.move_to(id, bucket)?;
        }
        Ok(Some(self.bare_item(&moved)))
    }

    fn finish(&self, id: u64) -> Result<(), String> {
        if let Some(done) = self.done_bucket {
            return self.move_to(id, done);
        }
        let mut raw = self.client.get_task_raw(id)?;
        raw.as_object_mut()
            .ok_or_else(|| t(msg::RESPONSE_UNREADABLE))?
            .insert("done".into(), json!(true));
        self.client.update_task(id, &raw).map(drop)
    }
}

impl Vikunja {
    fn target(&self) -> Result<&Target, String> {
        self.target.as_ref().ok_or_else(|| t(msg::NOT_CONFIGURED))
    }

    fn reorder(&mut self, id: u64, args: &Value) -> Result<(), String> {
        let wanted = args
            .get("position")
            .and_then(Value::as_u64)
            .ok_or_else(|| t(msg::POSITION_REQUIRED))?;
        let target = self.target.as_ref().ok_or_else(|| t(msg::NOT_CONFIGURED))?;
        let listed = |last: &Option<Snapshot>| {
            last.as_ref()
                .is_some_and(|s| s.entries.iter().any(|e| e.task.id == id))
        };
        if !listed(&self.last) {
            self.last = Some(target.snapshot()?);
        }
        let entries = match &mut self.last {
            Some(snapshot) if snapshot.entries.iter().any(|e| e.task.id == id) => {
                &mut snapshot.entries
            }
            _ => {
                return Err(t_args(
                    msg::TASK_NOT_LISTED,
                    &[("id", id.to_string().as_str().into())],
                ));
            }
        };
        let others: Vec<f64> = entries
            .iter()
            .filter(|e| e.task.id != id)
            .map(|e| e.task.position)
            .collect();
        let position = place_between(&others, usize::try_from(wanted).unwrap_or(usize::MAX));
        target.client.set_position(target.view, id, position)?;
        if let Some(entry) = entries.iter_mut().find(|e| e.task.id == id) {
            entry.task.position = position;
        }
        entries.sort_by(|a, b| a.task.position.total_cmp(&b.task.position));
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
        let base = checked_base(&cfg.url).map_err(|e| {
            t(match e {
                UrlError::Invalid => msg::URL_INVALID,
                UrlError::Insecure => msg::URL_INSECURE,
            })
        })?;
        let token = read_secret(&cfg.token_file).map_err(|e| {
            let id = match e {
                SecretError::Unreadable => msg::TOKEN_UNREADABLE,
                SecretError::Empty => msg::TOKEN_EMPTY,
            };
            t_args(id, &[("path", cfg.token_file.as_str().into())])
        })?;
        let client = Client::new(&base, &token)?;
        let project =
            by_title(client.projects()?, &cfg.project, |p| &p.title).ok_or_else(|| {
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
            base,
            from_prefix: cfg
                .from_label_prefix
                .unwrap_or_else(|| DEFAULT_FROM_LABEL_PREFIX.into()),
            project: project.id,
            view: view.id,
            default_bucket: Some(view.default_bucket_id).filter(|id| *id != 0),
            done_bucket: Some(view.done_bucket_id).filter(|id| *id != 0),
            bucket,
        });
        self.last = None;
        Ok(())
    }

    fn list(&mut self, _p: ListParams) -> Result<Vec<Item>, String> {
        let target = self.target.as_ref().ok_or_else(|| t(msg::NOT_CONFIGURED))?;
        let snapshot = self.last.insert(target.snapshot()?);
        let now = Utc::now();
        Ok(snapshot
            .entries
            .iter()
            .map(|e| target.item(e, now))
            .collect())
    }

    fn action(&mut self, p: ActionParams) -> Result<Option<Item>, String> {
        let id = parse_id(&p.item)?;
        let bucket = p.args.get("bucket").and_then(Value::as_str);
        if matches!((p.action.as_str(), bucket), ("reorder", _) | ("move", None)) {
            return self.reorder(id, &p.args).map(|()| None);
        }
        // a request that fails here may still have gone through on the server
        self.last = None;
        let target = self.target()?;
        match (p.action.as_str(), bucket) {
            ("done", _) => target.finish(id)?,
            ("delete", _) => target.client.delete_task(id)?,
            ("move", Some(title)) => target.move_to(id, target.bucket_by_title(title)?)?,
            (other, _) => return Err(t_args(msg::UNKNOWN_ACTION, &[("action", other.into())])),
        }
        Ok(None)
    }

    fn drop_item(&mut self, p: DropParams) -> Result<Item, String> {
        self.last = None;
        let target = self.target()?;
        if let Some(origin) = p.from.as_ref().filter(|s| s.plugin == "vikunja")
            && let Some(item) = target.adopt(&origin.id, p.item.url.as_deref())?
        {
            return Ok(item);
        }
        let source = p.item.source.as_ref().or(p.from.as_ref());
        target.create(&p.item, source)
    }

    // reorder positions come from what the last list showed, so a reminder check may read that snapshot but not replace it
    fn reminders(&mut self, p: RemindersParams) -> Result<Vec<Reminder>, String> {
        let target = self.target.as_ref().ok_or_else(|| t(msg::NOT_CONFIGURED))?;
        let fetched;
        let snapshot = match &self.last {
            Some(snapshot) if snapshot.at.elapsed() < SNAPSHOT_TTL => snapshot,
            _ => {
                fetched = target.snapshot()?;
                &fetched
            }
        };
        let now = Utc::now();
        let mut found = Vec::new();
        for entry in &snapshot.entries {
            for reminder in &entry.task.reminders {
                if let Some(at) = parse_time(&reminder.reminder).filter(|at| *at > p.since) {
                    found.push(Reminder {
                        at,
                        item: target.item(entry, now),
                    });
                }
            }
        }
        found.sort_by_key(|r| r.at);
        Ok(found)
    }
}

fn by_title<T>(mut items: Vec<T>, wanted: &str, title: impl Fn(&T) -> &str) -> Option<T> {
    let exact = items.iter().position(|i| title(i) == wanted);
    let at = exact.or_else(|| {
        let wanted = wanted.to_lowercase();
        items.iter().position(|i| title(i).to_lowercase() == wanted)
    })?;
    Some(items.swap_remove(at))
}

fn find_bucket(client: &Client, project: u64, view: u64, title: &str) -> Result<u64, String> {
    by_title(client.buckets(project, view)?, title, |b| &b.title)
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

// description_html defuses the marker in the text it wraps and writes ours last, so quoted text cannot pose as the source
fn parse_source(description: &str) -> Option<Source> {
    let start = description.rfind(SOURCE_MARKER)? + SOURCE_MARKER.len();
    let token = description[start..]
        .split(|c: char| c.is_whitespace() || c == '<')
        .next()?;
    let (plugin, id) = token.split_once(':')?;
    let clean = !plugin.is_empty() && !id.is_empty() && printable(token).len() == token.len();
    clean.then(|| Source {
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

fn is_safe_link(url: &str) -> bool {
    LINK_SCHEMES.iter().any(|scheme| {
        url.get(..scheme.len())
            .is_some_and(|head| head.eq_ignore_ascii_case(scheme))
    })
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
            .map(|l| {
                if is_safe_link(&l.url) {
                    format!("<a href=\"{}\">{}</a>", escape(&l.url), escape(&l.label))
                } else {
                    escape(&l.label)
                }
            })
            .collect();
        html.push_str(&format!("<p>{}</p>", links.join(" ")));
    }
    html = html.replace(SOURCE_MARKER, "focusdeck-source&#58; ");
    if let Some(source) = source {
        html.push_str(&format!(
            "<p>{SOURCE_MARKER}{}</p>",
            escape(&format!("{}:{}", source.plugin, source.id))
        ));
    }
    html
}

fn to_item(
    task: &Task,
    base: &str,
    from_prefix: &str,
    column: Option<&str>,
    now: DateTime<Utc>,
) -> Item {
    let due = task.due_date.as_deref().and_then(parse_time);
    let mut from = None;
    let mut labels = Vec::with_capacity(task.labels.len());
    for label in &task.labels {
        let title = printable(&label.title);
        let sender = (!from_prefix.is_empty())
            .then(|| title.strip_prefix(from_prefix))
            .flatten();
        match sender {
            Some(name) => {
                from.get_or_insert_with(|| name.to_string());
            }
            None => labels.push(title),
        }
    }
    let mut parts = Vec::new();
    if let Some(from) = &from {
        parts.push(format!("@{from}"));
    }
    parts.extend(column.filter(|c| !c.is_empty()).map(str::to_string));
    Item {
        id: task.id.to_string(),
        title: printable(&task.title),
        subtitle: (!parts.is_empty()).then(|| parts.join(" · ")),
        url: Some(format!("{base}/tasks/{}", task.id)),
        due,
        overdue: due.is_some_and(|d| d < now),
        source: parse_source(&task.description),
        description: None,
        labels,
        from,
        links: vec![],
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    proptest! {
        #[test]
        fn source_parsing_survives_any_description(description in "\\PC*|.*") {
            let _ = parse_source(&description);
        }

        #[test]
        fn a_written_source_reads_back_whatever_the_text_above_it_says(
            text in "\\PC*|\\PC{0,8}focusdeck-source: [a-z]{1,5}:[0-9]{1,5}\\PC{0,8}",
            plugin in "[a-z][a-z0-9-]{0,11}",
            id in "[A-Za-z0-9_./-]{1,24}",
        ) {
            let item = Item {
                id: String::new(),
                title: String::new(),
                subtitle: None,
                url: None,
                due: None,
                overdue: false,
                source: None,
                description: Some(text),
                labels: vec![],
                from: None,
                links: vec![],
            };
            let source = Source { plugin, id };
            let html = description_html(&item, Some(&source));
            prop_assert_eq!(parse_source(&html), Some(source));
            prop_assert_eq!(parse_source(&description_html(&item, None)), None);
        }

        #[test]
        fn a_reordered_task_lands_strictly_between_its_neighbours(
            mut others in prop::collection::vec(-1.0e9f64..1.0e9, 0..40),
            wanted in 0usize..60,
        ) {
            others.sort_by(f64::total_cmp);
            others.dedup();
            let position = place_between(&others, wanted);
            prop_assert!(position.is_finite());
            let at = wanted.min(others.len());
            if let Some(above) = at.checked_sub(1).map(|i| others[i]) {
                prop_assert!(above < position, "{above} !< {position}");
            }
            if let Some(below) = others.get(at) {
                prop_assert!(position < *below, "{position} !< {below}");
            }
        }

        #[test]
        fn only_listed_schemes_become_links(scheme in "[a-zA-Z][a-zA-Z0-9+.-]{0,10}", rest in "\\PC{0,20}") {
            let url = format!("{scheme}:{rest}");
            let allowed = ["http", "https", "mailto"].contains(&scheme.to_ascii_lowercase().as_str());
            if is_safe_link(&url) {
                prop_assert!(allowed, "{url}");
            }
        }
    }
}
