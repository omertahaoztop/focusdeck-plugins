use focusdeck_i18n::Localizer;
use focusdeck_protocol::{ActionParams, Describe, Item, ListParams, PROTOCOL_VERSION, Plugin};
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
        let key = std::fs::read_to_string(&cfg.key_file).map_err(|_| {
            t_args(
                msg::KEY_FILE_UNREADABLE,
                &[("path", cfg.key_file.as_str().into())],
            )
        })?;
        let key = key.trim();
        if key.is_empty() {
            return Err(t_args(
                msg::KEY_FILE_EMPTY,
                &[("path", cfg.key_file.as_str().into())],
            ));
        }
        let client = Client::new(&cfg.url, &cfg.email, key)?;
        client.me()?;
        self.target = Some(Target {
            client,
            base: cfg.url,
        });
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
    let subtitle = if message.kind == "private" {
        localizer.t_args(
            msg::PRIVATE_SUBTITLE,
            &[("sender", message.sender_full_name.as_str().into())],
        )
    } else {
        let stream = message.display_recipient.as_str().unwrap_or_default();
        format!(
            "{} · #{stream} > {}",
            message.sender_full_name, message.subject
        )
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
        .map(|line| line.trim_start_matches(|c: char| c == '>' || c.is_whitespace()))
        .map(str::trim_end)
        .find(|line| !line.is_empty())
        .map_or_else(|| localizer.t(msg::EMPTY_TITLE), str::to_string)
}
