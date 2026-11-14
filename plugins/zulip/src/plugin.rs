use seenboard_i18n::Localizer;
use seenboard_plugin_common::{SecretError, UrlError, checked_base, printable, read_secret};
use seenboard_protocol::{ActionParams, Describe, Item, ListParams, PROTOCOL_VERSION, Plugin};
use serde::Deserialize;
use serde_json::Value;

use crate::client::{Client, Message};
use crate::i18n::{self, msg, t, t_args};

#[derive(Deserialize)]
struct Config {
    url: String,
    email: String,
    key_file: String,
}

#[derive(Default)]
pub struct Zulip {
    target: Option<Target>,
}

struct Target {
    client: Client,
    base: String,
}

impl Plugin for Zulip {
    fn describe(&self) -> Describe {
        Describe {
            name: "zulip".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            protocol: PROTOCOL_VERSION,
            actions: vec!["unstar".into()],
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
        let key = read_secret(&cfg.key_file).map_err(|e| {
            let id = match e {
                SecretError::Unreadable => msg::KEY_FILE_UNREADABLE,
                SecretError::Empty => msg::KEY_FILE_EMPTY,
            };
            t_args(id, &[("path", cfg.key_file.as_str().into())])
        })?;
        let client = Client::new(&base, &cfg.email, &key)?;
        client.me()?;
        self.target = Some(Target { client, base });
        Ok(())
    }

    fn list(&mut self, _p: ListParams) -> Result<Vec<Item>, String> {
        let target = self.target.as_ref().ok_or_else(|| t(msg::NOT_CONFIGURED))?;
        Ok(target
            .client
            .starred()?
            .iter()
            .map(|msg| to_item(msg, &target.base))
            .collect())
    }

    fn action(&mut self, p: ActionParams) -> Result<Option<Item>, String> {
        match p.action.as_str() {
            "unstar" => {
                let target = self.target.as_ref().ok_or_else(|| t(msg::NOT_CONFIGURED))?;
                let id: u64 = p
                    .item
                    .parse()
                    .map_err(|_| t_args(msg::INVALID_ID, &[("id", p.item.as_str().into())]))?;
                target.client.unstar(id)?;
                Ok(None)
            }
            other => Err(t_args(msg::UNKNOWN_ACTION, &[("action", other.into())])),
        }
    }
}

pub fn to_item(message: &Message, base: &str) -> Item {
    to_item_in(i18n::current(), message, base)
}

pub fn to_item_in(localizer: &Localizer, message: &Message, base: &str) -> Item {
    let base = base.trim_end_matches('/');
    let sender = printable(&message.sender_full_name);
    let subtitle = if message.kind == "private" {
        localizer.t_args(msg::PRIVATE_SUBTITLE, &[("sender", sender.as_str().into())])
    } else {
        let stream = printable(message.display_recipient.as_str().unwrap_or_default());
        format!("{sender} · #{stream} > {}", printable(&message.subject))
    };
    Item {
        id: message.id.to_string(),
        title: title_of(localizer, &message.content),
        subtitle: Some(subtitle),
        url: Some(format!("{base}/#narrow/id/{}", message.id)),
        due: None,
        overdue: false,
        source: None,
        description: None,
        labels: vec![],
        from: None,
        links: vec![],
    }
}

fn title_of(localizer: &Localizer, content: &str) -> String {
    content
        .lines()
        .map(|line| printable(line.trim_start_matches(|c: char| c == '>' || c.is_whitespace())))
        .find(|line| !line.is_empty())
        .unwrap_or_else(|| localizer.t(msg::EMPTY_TITLE))
}
