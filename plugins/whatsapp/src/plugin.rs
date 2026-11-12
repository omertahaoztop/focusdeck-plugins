use chrono::{DateTime, Duration, Local, NaiveDate, TimeZone, Utc};
use focusdeck_i18n::Localizer;
use focusdeck_plugin_common::{SecretError, UrlError, checked_base, printable, read_secret};
use focusdeck_protocol::{ActionParams, Describe, Item, ListParams, PROTOCOL_VERSION, Plugin};
use serde::Deserialize;
use serde_json::Value;

use crate::client::{Client, Message};
use crate::i18n::{self, msg, t, t_args};

#[derive(Deserialize)]
struct Config {
    url: String,
    user: String,
    key_file: String,
    jid: String,
    since: Option<String>,
}

#[derive(Default)]
pub struct WhatsApp {
    client: Option<Client>,
}

impl WhatsApp {
    fn client(&self) -> Result<&Client, String> {
        self.client.as_ref().ok_or_else(|| t(msg::NOT_CONFIGURED))
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
        let password = read_secret(&cfg.key_file).map_err(|e| {
            let id = match e {
                SecretError::Unreadable => msg::KEY_FILE_UNREADABLE,
                SecretError::Empty => msg::KEY_FILE_EMPTY,
            };
            t_args(id, &[("path", cfg.key_file.as_str().into())])
        })?;
        let client = Client::new(&base, &cfg.user, &password, &cfg.jid, since)?;
        client.probe()?;
        self.client = Some(client);
        Ok(())
    }

    fn list(&mut self, _p: ListParams) -> Result<Vec<Item>, String> {
        let mut messages = self.client()?.messages()?;
        messages.sort_by_key(|m| std::cmp::Reverse(m.timestamp));
        let now = Utc::now();
        let localizer = i18n::current();
        Ok(messages
            .iter()
            .filter(|m| !m.media_type.eq_ignore_ascii_case("call"))
            .map(|m| to_item_in(localizer, m, now))
            .collect())
    }

    fn action(&mut self, p: ActionParams) -> Result<Option<Item>, String> {
        match p.action.as_str() {
            "done" => {
                self.client()?.delete(&p.item)?;
                Ok(None)
            }
            other => Err(t_args(msg::UNKNOWN_ACTION, &[("action", other.into())])),
        }
    }
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
    let day = NaiveDate::parse_from_str(raw.trim(), "%Y-%m-%d").ok();
    // a zone that skips midnight on this date starts the day at its first existing hour
    day.and_then(|day| {
        (0..24).find_map(|hour| {
            let start = day.and_hms_opt(hour, 0, 0)?;
            start.and_local_timezone(tz.clone()).earliest()
        })
    })
    .map(|start| start.with_timezone(&Utc))
    .ok_or_else(|| t(msg::SINCE_INVALID))
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
