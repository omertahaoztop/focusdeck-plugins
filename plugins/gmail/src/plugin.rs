use chrono::{DateTime, Duration, Local, Utc};
use focusdeck_protocol::{ActionParams, Describe, Item, ListParams, PROTOCOL_VERSION, Plugin};
use serde::Deserialize;
use serde_json::Value;

use crate::client::{Client, Message};
use crate::oauth::{read_client_file, read_refresh_token};

const DEFAULT_API: &str = "https://gmail.googleapis.com/gmail/v1/users/me";
const NOT_CONFIGURED: &str = "Gmail ayarlanmadı";
const NO_SUBJECT: &str = "(konu yok)";
const NO_SENDER: &str = "(gönderen yok)";

#[derive(Deserialize)]
struct Config {
    client_file: String,
    token_file: String,
    api_url: Option<String>,
}

#[derive(Default)]
pub struct Gmail {
    client: Option<Client>,
}

impl Plugin for Gmail {
    fn describe(&self) -> Describe {
        Describe {
            name: "gmail".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            protocol: PROTOCOL_VERSION,
            actions: vec![],
            accepts_drop: false,
            reminders: false,
        }
    }

    fn configure(&mut self, config: Value) -> Result<(), String> {
        let cfg: Config =
            serde_json::from_value(config).map_err(|e| format!("Ayar hatalı: {e}"))?;
        let secret = read_client_file(cfg.client_file.as_ref())?;
        let refresh_token = read_refresh_token(cfg.token_file.as_ref())?;
        let api = cfg.api_url.as_deref().unwrap_or(DEFAULT_API);
        let mut client = Client::new(api, secret, refresh_token)?;
        client.authorize()?;
        self.client = Some(client);
        Ok(())
    }

    fn list(&mut self, _p: ListParams) -> Result<Vec<Item>, String> {
        let client = self.client.as_mut().ok_or(NOT_CONFIGURED)?;
        let now = Local::now();
        Ok(client
            .starred()?
            .iter()
            .map(|msg| to_item(msg, now))
            .collect())
    }

    fn action(&mut self, _p: ActionParams) -> Result<Option<Item>, String> {
        Err("bu kutuda eylem yok".into())
    }
}

pub fn to_item(msg: &Message, now: DateTime<Local>) -> Item {
    let sent = DateTime::<Utc>::from_timestamp_millis(msg.internal_date_ms)
        .unwrap_or(DateTime::<Utc>::UNIX_EPOCH);
    let age = now.signed_duration_since(sent).max(Duration::zero());
    let subject = printable(&msg.subject);
    Item {
        id: msg.id.clone(),
        title: if subject.is_empty() {
            NO_SUBJECT.into()
        } else {
            subject
        },
        subtitle: Some(format!("{} · {}", sender_label(&msg.from), age_label(age))),
        url: Some(format!(
            "https://mail.google.com/mail/#all/{}",
            msg.thread_id
        )),
        due: None,
        overdue: age >= Duration::days(3),
        source: None,
        description: None,
        labels: vec![],
        from: None,
        links: vec![],
    }
}

fn age_label(age: Duration) -> String {
    let minutes = age.num_minutes();
    if minutes < 60 {
        format!("{minutes}dk")
    } else if minutes < 24 * 60 {
        format!("{}s", minutes / 60)
    } else {
        format!("{}g", minutes / (24 * 60))
    }
}

fn sender_label(from: &str) -> String {
    let from = printable(from);
    let (name, address) = match (from.rfind('<'), from.rfind('>')) {
        (Some(open), Some(close)) if open < close => (&from[..open], from[open + 1..close].trim()),
        _ => ("", from.as_str()),
    };
    let name = name.trim().trim_matches('"').trim();
    if !name.is_empty() {
        name.to_string()
    } else if !address.is_empty() {
        address.to_string()
    } else {
        NO_SENDER.to_string()
    }
}

fn printable(text: &str) -> String {
    text.chars()
        .filter(|c| !c.is_control())
        .collect::<String>()
        .trim()
        .to_string()
}
