use std::ffi::OsString;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Duration, Local, NaiveDate, TimeZone, Utc};
use focusdeck_i18n::Localizer;
use focusdeck_plugin_common::{SecretError, UrlError, checked_base, printable, read_secret};
use focusdeck_protocol::{ActionParams, Describe, Item, ListParams, PROTOCOL_VERSION, Plugin};
use serde::Deserialize;
use serde_json::Value;

use crate::client::{Client, Message};
use crate::hidden::{Hidden, HiddenError};
use crate::i18n::{self, msg, t, t_args};

#[derive(Deserialize)]
struct Config {
    url: String,
    user: String,
    key_file: String,
    jid: String,
    since: Option<String>,
}

pub struct WhatsApp {
    state_dir: Option<PathBuf>,
    target: Option<Target>,
}

struct Target {
    client: Client,
    hidden_path: PathBuf,
}

impl Default for WhatsApp {
    fn default() -> Self {
        Self {
            state_dir: state_dir_from_env(),
            target: None,
        }
    }
}

fn state_dir_from_env() -> Option<PathBuf> {
    state_dir_from(|name| std::env::var_os(name))
}

// same order as the core: FOCUSDECK_STATE_DIR, then the XDG state home under a focusdeck folder
pub fn state_dir_from(get: impl Fn(&str) -> Option<OsString>) -> Option<PathBuf> {
    let var = |name: &str| get(name).filter(|v| !v.is_empty());
    if let Some(dir) = var("FOCUSDECK_STATE_DIR") {
        return Some(PathBuf::from(dir));
    }
    let base = var("XDG_STATE_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| var("HOME").map(|h| PathBuf::from(h).join(".local/state")))?;
    Some(base.join("focusdeck"))
}

impl WhatsApp {
    pub fn with_state_dir(dir: PathBuf) -> Self {
        Self {
            state_dir: Some(dir),
            target: None,
        }
    }

    fn target(&self) -> Result<&Target, String> {
        self.target.as_ref().ok_or_else(|| t(msg::NOT_CONFIGURED))
    }
}

impl Plugin for WhatsApp {
    fn describe(&self) -> Describe {
        Describe {
            name: "whatsapp".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            protocol: PROTOCOL_VERSION,
            actions: vec!["done".into()],
            accepts_drop: false,
            reminders: false,
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
        if !valid_jid(&cfg.jid) {
            return Err(t(msg::JID_INVALID));
        }
        let since = cfg
            .since
            .as_deref()
            .map(|raw| parse_since_in(raw, &Local))
            .transpose()?;
        let state_dir = self
            .state_dir
            .clone()
            .ok_or_else(|| t(msg::STATE_DIR_UNKNOWN))?;
        let password = read_secret(&cfg.key_file).map_err(|e| {
            let id = match e {
                SecretError::Unreadable => msg::KEY_FILE_UNREADABLE,
                SecretError::Empty => msg::KEY_FILE_EMPTY,
            };
            t_args(id, &[("path", cfg.key_file.as_str().into())])
        })?;
        let client = Client::new(&base, &cfg.user, &password, &cfg.jid, since)?;
        client.probe()?;
        self.target = Some(Target {
            client,
            hidden_path: hidden_path_in(&state_dir, &cfg.jid),
        });
        Ok(())
    }

    fn list(&mut self, _p: ListParams) -> Result<Vec<Item>, String> {
        let target = self.target()?;
        let mut hidden = load_hidden(&target.hidden_path)?;
        let mut messages = target.client.messages()?;
        let prune = target.client.since().is_none() && !messages.is_empty();
        if prune && hidden.retain(messages.iter().map(|m| m.id.as_str())) {
            save_hidden(&hidden, &target.hidden_path)?;
        }
        messages.sort_by_key(|m| std::cmp::Reverse(m.timestamp));
        let now = Utc::now();
        let localizer = i18n::current();
        Ok(messages
            .iter()
            .filter(|m| !m.media_type.eq_ignore_ascii_case("call") && !hidden.contains(&m.id))
            .map(|m| to_item_in(localizer, m, now))
            .collect())
    }

    fn action(&mut self, p: ActionParams) -> Result<Option<Item>, String> {
        match p.action.as_str() {
            "done" => {
                let target = self.target()?;
                let mut hidden = load_hidden(&target.hidden_path)?;
                if hidden.insert(&p.item) {
                    save_hidden(&hidden, &target.hidden_path)?;
                }
                Ok(None)
            }
            other => Err(t_args(msg::UNKNOWN_ACTION, &[("action", other.into())])),
        }
    }
}

// the jid passed validation, so it is a single safe path component
pub fn hidden_path_in(state_dir: &Path, jid: &str) -> PathBuf {
    state_dir.join(format!("whatsapp-hidden-{jid}.json"))
}

fn valid_jid(jid: &str) -> bool {
    let Some((user, server)) = jid.split_once('@') else {
        return false;
    };
    !user.is_empty()
        && user
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | ':' | '-'))
        && matches!(server, "s.whatsapp.net" | "lid")
}

pub fn parse_since_in<Tz: TimeZone>(raw: &str, tz: &Tz) -> Result<DateTime<Utc>, String> {
    NaiveDate::parse_from_str(raw.trim(), "%Y-%m-%d")
        .ok()
        .and_then(|day| day.and_hms_opt(0, 0, 0))
        .and_then(|start| start.and_local_timezone(tz.clone()).earliest())
        .map(|start| start.with_timezone(&Utc))
        .ok_or_else(|| t(msg::SINCE_INVALID))
}

fn load_hidden(path: &Path) -> Result<Hidden, String> {
    Hidden::load(path).map_err(|e| hidden_error(e, path))
}

fn save_hidden(hidden: &Hidden, path: &Path) -> Result<(), String> {
    hidden.save().map_err(|e| hidden_error(e, path))
}

fn hidden_error(error: HiddenError, path: &Path) -> String {
    let id = match error {
        HiddenError::Unreadable => msg::HIDDEN_UNREADABLE,
        HiddenError::Unwritable => msg::HIDDEN_UNWRITABLE,
    };
    t_args(id, &[("path", path.display().to_string().as_str().into())])
}

pub fn to_item_in(localizer: &Localizer, message: &Message, now: DateTime<Utc>) -> Item {
    let mut lines: Vec<String> = message.content.lines().map(printable).collect();
    while lines.last().is_some_and(String::is_empty) {
        lines.pop();
    }
    let lead = lines.iter().take_while(|l| l.is_empty()).count();
    lines.drain(..lead);
    let title = lines
        .iter()
        .find(|l| !l.is_empty())
        .cloned()
        .unwrap_or_else(|| placeholder(localizer, message));
    let age = now
        .signed_duration_since(message.timestamp)
        .max(Duration::zero());
    Item {
        id: message.id.clone(),
        title,
        subtitle: Some(age_label(localizer, age)),
        url: first_link(&message.content),
        due: None,
        overdue: false,
        source: None,
        description: (lines.len() > 1).then(|| lines.join("\n")),
        labels: vec![],
        from: None,
        links: vec![],
    }
}

fn placeholder(localizer: &Localizer, message: &Message) -> String {
    let name = printable(&message.filename);
    match message.media_type.to_ascii_lowercase().as_str() {
        "" => localizer.t(msg::EMPTY_TITLE),
        "image" => localizer.t(msg::TITLE_IMAGE),
        "video" => localizer.t(msg::TITLE_VIDEO),
        "audio" => localizer.t(msg::TITLE_AUDIO),
        "document" if name.is_empty() => localizer.t(msg::TITLE_DOCUMENT_UNNAMED),
        "document" => localizer.t_args(msg::TITLE_DOCUMENT, &[("name", name.as_str().into())]),
        _ => localizer.t(msg::TITLE_MEDIA),
    }
}

fn first_link(content: &str) -> Option<String> {
    content
        .split_whitespace()
        .filter(|w| w.starts_with("http://") || w.starts_with("https://"))
        .map(|w| w.trim_end_matches(['.', ',', ';', ':', '!', '?', ')', ']', '>', '"', '\'']))
        .map(printable)
        .find(|w| reqwest::Url::parse(w).is_ok_and(|u| u.host_str().is_some()))
}

fn age_label(localizer: &Localizer, age: Duration) -> String {
    let minutes = age.num_minutes();
    if minutes < 60 {
        localizer.t_args(msg::AGE_MINUTES, &[("minutes", minutes.into())])
    } else if minutes < 24 * 60 {
        localizer.t_args(msg::AGE_HOURS, &[("hours", (minutes / 60).into())])
    } else {
        localizer.t_args(msg::AGE_DAYS, &[("days", (minutes / (24 * 60)).into())])
    }
}
