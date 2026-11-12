use chrono::{Local, NaiveDate};
use focusdeck_i18n::Localizer;
use focusdeck_protocol::{ActionParams, Describe, Item, ListParams, PROTOCOL_VERSION, Plugin};
use serde::Deserialize;
use serde_json::Value;

use crate::client::{Client, Issue};
use crate::i18n::{self, msg, t, t_args};

const DUE_HOUR: u32 = 17;

#[derive(Deserialize)]
struct Config {
    url: String,
    key_file: String,
}

#[derive(Default)]
pub struct Redmine {
    target: Option<Target>,
}

struct Target {
    client: Client,
    base: String,
}

impl Plugin for Redmine {
    fn describe(&self) -> Describe {
        Describe {
            name: "redmine".into(),
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
        let client = Client::new(&cfg.url, key)?;
        client.current_user()?;
        self.target = Some(Target {
            client,
            base: cfg.url,
        });
        Ok(())
    }

    fn list(&mut self, _p: ListParams) -> Result<Vec<Item>, String> {
        let target = self.target.as_ref().ok_or_else(|| t(msg::NOT_CONFIGURED))?;
        let today = Local::now().date_naive();
        Ok(target
            .client
            .issues()?
            .iter()
            .map(|issue| to_item(issue, &target.base, today))
            .collect())
    }

    fn action(&mut self, _p: ActionParams) -> Result<Option<Item>, String> {
        Err(t(msg::NO_ACTIONS))
    }
}

pub fn late_subtitle(localizer: &Localizer, days: i64) -> String {
    localizer.t_args(msg::LATE, &[("days", days.into())])
}

pub fn to_item(issue: &Issue, base: &str, today: NaiveDate) -> Item {
    let due_day = issue
        .due_date
        .as_deref()
        .and_then(|d| NaiveDate::parse_from_str(d, "%Y-%m-%d").ok());
    let due = due_day
        .and_then(|day| day.and_hms_opt(DUE_HOUR, 0, 0))
        .and_then(|time| time.and_local_timezone(Local).earliest())
        .map(|time| time.fixed_offset());
    let late_days = due_day
        .map(|day| (today - day).num_days())
        .filter(|days| *days > 0);
    let priority = issue
        .priority
        .as_ref()
        .and_then(|p| p.name.split(" - ").next())
        .map(str::trim)
        .filter(|name| !name.is_empty());
    let mut parts: Vec<String> = priority.map(str::to_string).into_iter().collect();
    parts.extend(late_days.map(|days| late_subtitle(i18n::current(), days)));
    Item {
        id: issue.id.to_string(),
        title: issue.subject.clone(),
        subtitle: (!parts.is_empty()).then(|| parts.join(" · ")),
        url: Some(format!(
            "{}/issues/{}",
            base.trim_end_matches('/'),
            issue.id
        )),
        due,
        overdue: late_days.is_some(),
        source: None,
        description: None,
        labels: vec![],
        from: None,
        links: vec![],
    }
}
