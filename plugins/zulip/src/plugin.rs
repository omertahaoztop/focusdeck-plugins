use focusdeck_protocol::{ActionParams, Describe, Item, ListParams, PROTOCOL_VERSION, Plugin};
use serde::Deserialize;
use serde_json::Value;

use crate::client::{Client, Message};

const NOT_CONFIGURED: &str = "Zulip ayarlanmadı";
const EMPTY_TITLE: &str = "(boş mesaj)";

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
        let cfg: Config =
            serde_json::from_value(config).map_err(|e| format!("Ayar hatalı: {e}"))?;
        let key = std::fs::read_to_string(&cfg.key_file)
            .map_err(|_| format!("Zulip anahtar dosyası okunamadı: {}", cfg.key_file))?;
        let key = key.trim();
        if key.is_empty() {
            return Err(format!("Zulip anahtar dosyası boş: {}", cfg.key_file));
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
        let target = self.target.as_ref().ok_or(NOT_CONFIGURED)?;
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
                let target = self.target.as_ref().ok_or(NOT_CONFIGURED)?;
                let id: u64 = p
                    .item
                    .parse()
                    .map_err(|_| format!("Geçersiz kimlik: {}", p.item))?;
                target.client.unstar(id)?;
                Ok(None)
            }
            other => Err(format!("Bilinmeyen eylem: {other}")),
        }
    }
}

pub fn to_item(msg: &Message, base: &str) -> Item {
    let base = base.trim_end_matches('/');
    let subtitle = if msg.kind == "private" {
        format!("{} · özel", msg.sender_full_name)
    } else {
        let stream = msg.display_recipient.as_str().unwrap_or_default();
        format!("{} · #{stream} > {}", msg.sender_full_name, msg.subject)
    };
    Item {
        id: msg.id.to_string(),
        title: title_of(&msg.content),
        subtitle: Some(subtitle),
        url: Some(format!("{base}/#narrow/id/{}", msg.id)),
        due: None,
        overdue: false,
        source: None,
        description: None,
        labels: vec![],
        from: None,
        links: vec![],
    }
}

fn title_of(content: &str) -> String {
    content
        .lines()
        .map(|line| line.trim_start_matches(|c: char| c == '>' || c.is_whitespace()))
        .map(str::trim_end)
        .find(|line| !line.is_empty())
        .unwrap_or(EMPTY_TITLE)
        .to_string()
}
