use chrono::{DateTime, Duration, Local, Utc};
use focusdeck_i18n::Localizer;
use focusdeck_plugin_common::{checked_url, printable};
use focusdeck_protocol::{ActionParams, Describe, Item, ListParams, PROTOCOL_VERSION, Plugin};
use serde::Deserialize;
use serde_json::Value;

use crate::client::{Client, Message, url_error};
use crate::i18n::{self, msg, t, t_args};
use crate::oauth::{read_client_file, read_refresh_token};

const DEFAULT_API: &str = "https://gmail.googleapis.com/gmail/v1/users/me";

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
        let cfg: Config = serde_json::from_value(config).map_err(|e| {
            t_args(
                msg::INVALID_SETTINGS,
                &[("error", e.to_string().as_str().into())],
            )
        })?;
        let mut api = checked_url(cfg.api_url.as_deref().unwrap_or(DEFAULT_API))
            .map_err(|e| url_error("api_url", e))?;
        let path = api.path().trim_end_matches('/').to_string();
        api.set_path(&path);
        let secret = read_client_file(cfg.client_file.as_ref())?;
        let refresh_token = read_refresh_token(cfg.token_file.as_ref())?;
        let mut client = Client::new(api, secret, refresh_token)?;
        client.authorize()?;
        self.client = Some(client);
        Ok(())
    }

    fn list(&mut self, _p: ListParams) -> Result<Vec<Item>, String> {
        let client = self.client.as_mut().ok_or_else(|| t(msg::NOT_CONFIGURED))?;
        let now = Local::now();
        Ok(client
            .starred()?
            .into_iter()
            .map(|message| to_item(message, now))
            .collect())
    }

    fn action(&mut self, _p: ActionParams) -> Result<Option<Item>, String> {
        Err(t(msg::NO_ACTIONS))
    }
}

pub fn to_item(message: &Message, now: DateTime<Local>) -> Item {
    let localizer = i18n::current();
    let sent = DateTime::<Utc>::from_timestamp_millis(message.internal_date_ms)
        .unwrap_or(DateTime::<Utc>::UNIX_EPOCH);
    let age = now.signed_duration_since(sent).max(Duration::zero());
    let subject = printable(&message.subject);
    let sender = sender_label(localizer, &message.from);
    Item {
        id: message.id.clone(),
        title: if subject.is_empty() {
            localizer.t(msg::NO_SUBJECT)
        } else {
            subject
        },
        subtitle: Some(format!("{sender} · {}", age_label(localizer, age))),
        url: Some(format!(
            "https://mail.google.com/mail/#all/{}",
            message.thread_id
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

pub fn age_label(localizer: &Localizer, age: Duration) -> String {
    let minutes = age.num_minutes();
    if minutes < 60 {
        localizer.t_args(msg::AGE_MINUTES, &[("minutes", minutes.into())])
    } else if minutes < 24 * 60 {
        localizer.t_args(msg::AGE_HOURS, &[("hours", (minutes / 60).into())])
    } else {
        localizer.t_args(msg::AGE_DAYS, &[("days", (minutes / (24 * 60)).into())])
    }
}

fn sender_label(localizer: &Localizer, from: &str) -> String {
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
        localizer.t(msg::NO_SENDER)
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    proptest! {
        #[test]
        fn sender_label_is_never_empty_and_never_carries_control_characters(from in any::<String>()) {
            let label = sender_label(i18n::current(), &from);
            prop_assert!(!label.is_empty());
            prop_assert!(!label.chars().any(char::is_control));
        }

        #[test]
        fn sender_label_prefers_the_display_name(name in "[a-zA-ZçğıöşüÇĞİÖŞÜ]{1,12}", user in "[a-z]{1,8}") {
            let from = format!("\"{name}\" <{user}@example.com>");
            prop_assert_eq!(sender_label(i18n::current(), &from), name);
        }
    }
}
