use std::io::Write;
use std::os::unix::fs::PermissionsExt;

use chrono::{Local, NaiveDate, TimeZone};
use focusdeck_protocol::{ActionParams, ListParams, PROTOCOL_VERSION, Plugin, View};
use focusdeck_redmine::client::Issue;
use focusdeck_redmine::i18n;
use focusdeck_redmine::plugin::{Redmine, late_subtitle, to_item};
use mockito::{Matcher, Mock, ServerGuard};
use serde_json::{Value, json};
use tempfile::NamedTempFile;

const KEY: &str = "s3cret-key";

fn keyfile(content: &str) -> NamedTempFile {
    let mut file = NamedTempFile::new().unwrap();
    write!(file, "{content}").unwrap();
    std::fs::set_permissions(file.path(), std::fs::Permissions::from_mode(0o600)).unwrap();
    file
}

fn config(server: &ServerGuard, keyfile: &NamedTempFile) -> Value {
    json!({"url": server.url(), "key_file": keyfile.path().to_str().unwrap()})
}

fn current_user(server: &mut ServerGuard) -> Mock {
    server
        .mock("GET", "/users/current.json")
        .match_header("x-redmine-api-key", KEY)
        .with_body(json!({"user": {"id": 1}}).to_string())
        .create()
}

fn configured(server: &mut ServerGuard) -> (Redmine, NamedTempFile, Mock) {
    let user = current_user(server);
    let file = keyfile(&format!("{KEY}\n"));
    let mut plugin = Redmine::default();
    plugin.configure(config(server, &file)).unwrap();
    (plugin, file, user)
}

fn issue_json(id: u64) -> Value {
    json!({"id": id, "subject": format!("iş {id}"), "priority": {"id": 5, "name": "Ö2 - Yüksek"}})
}

fn page(server: &mut ServerGuard, offset: u64, issues: Vec<Value>, total: u64) -> Mock {
    server
        .mock("GET", "/issues.json")
        .match_header("x-redmine-api-key", KEY)
        .match_query(Matcher::AllOf(vec![
            Matcher::UrlEncoded("assigned_to_id".into(), "me".into()),
            Matcher::UrlEncoded("status_id".into(), "open".into()),
            Matcher::UrlEncoded("sort".into(), "priority:desc,updated_on:desc".into()),
            Matcher::UrlEncoded("limit".into(), "100".into()),
            Matcher::UrlEncoded("offset".into(), offset.to_string()),
        ]))
        .with_body(
            json!({"issues": issues, "total_count": total, "offset": offset, "limit": 100})
                .to_string(),
        )
        .create()
}

fn list(plugin: &mut Redmine) -> Result<Vec<focusdeck_protocol::Item>, String> {
    plugin.list(ListParams {
        view: View::Summary,
    })
}

fn issue(due: Option<&str>, priority: &str) -> Issue {
    serde_json::from_value(json!({
        "id": 42, "subject": "Bir iş", "priority": {"id": 6, "name": priority}, "due_date": due,
    }))
    .unwrap()
}

fn day(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).unwrap()
}

#[test]
fn describe_matches_contract() {
    let d = Redmine::default().describe();
    assert_eq!(d.name, "redmine");
    assert_eq!(d.version, env!("CARGO_PKG_VERSION"));
    assert_eq!(d.protocol, PROTOCOL_VERSION);
    assert!(d.actions.is_empty());
    assert!(!d.accepts_drop && !d.reminders);
}

#[test]
fn action_is_always_rejected() {
    let err = Redmine::default()
        .action(ActionParams {
            item: "1".into(),
            action: "done".into(),
            args: Value::Null,
        })
        .unwrap_err();
    assert_eq!(err, "this box has no actions");
}

#[test]
fn lists_assigned_open_issues() {
    let mut server = mockito::Server::new();
    let (mut plugin, _file, _user) = configured(&mut server);
    let m = page(&mut server, 0, vec![issue_json(7), issue_json(3)], 2);
    let items = list(&mut plugin).unwrap();
    m.assert();
    let ids: Vec<_> = items.iter().map(|i| i.id.as_str()).collect();
    assert_eq!(ids, ["7", "3"]);
    assert_eq!(items[0].title, "iş 7");
    assert_eq!(items[0].url, Some(format!("{}/issues/7", server.url())));
    assert_eq!(items[0].subtitle.as_deref(), Some("Ö2"));
}

#[test]
fn pages_until_total_count() {
    let mut server = mockito::Server::new();
    let (mut plugin, _file, _user) = configured(&mut server);
    let first = page(&mut server, 0, (1..=100).map(issue_json).collect(), 150);
    let second = page(&mut server, 100, (101..=150).map(issue_json).collect(), 150);
    let items = list(&mut plugin).unwrap();
    first.assert();
    second.assert();
    assert_eq!(items.len(), 150);
}

#[test]
fn incomplete_listing_is_an_error() {
    let mut server = mockito::Server::new();
    let (mut plugin, _file, _user) = configured(&mut server);
    page(&mut server, 0, (1..=100).map(issue_json).collect(), 150);
    page(&mut server, 100, vec![], 150);
    let err = list(&mut plugin).unwrap_err();
    assert_eq!(err, "Redmine: list came back incomplete (100/150)");
}

#[test]
fn duplicate_across_pages_counts_once() {
    let mut server = mockito::Server::new();
    let (mut plugin, _file, _user) = configured(&mut server);
    page(&mut server, 0, vec![issue_json(1), issue_json(2)], 3);
    page(&mut server, 2, vec![issue_json(1)], 3);
    let err = list(&mut plugin).unwrap_err();
    assert_eq!(err, "Redmine: list came back incomplete (2/3)");
}

#[test]
fn http_error_names_only_the_status() {
    let mut server = mockito::Server::new();
    let (mut plugin, _file, _user) = configured(&mut server);
    server
        .mock("GET", "/issues.json")
        .match_query(Matcher::Any)
        .with_status(500)
        .with_body(format!("secret body {KEY}"))
        .create();
    let err = list(&mut plugin).unwrap_err();
    assert_eq!(err, "Redmine: HTTP 500");
}

#[test]
fn to_item_marks_overdue_with_days() {
    let item = to_item(
        &issue(Some("2026-11-08"), "Ö1 - Kritik Etki"),
        "https://redmine.example.com/",
        day(2026, 11, 12),
    );
    let expected = Local
        .with_ymd_and_hms(2026, 11, 8, 17, 0, 0)
        .earliest()
        .unwrap();
    assert_eq!(item.subtitle.as_deref(), Some("Ö1 · 4d overdue"));
    assert!(item.overdue);
    assert_eq!(item.due, Some(expected.fixed_offset()));
    assert_eq!(item.id, "42");
    assert_eq!(item.title, "Bir iş");
    assert_eq!(
        item.url.as_deref(),
        Some("https://redmine.example.com/issues/42")
    );
}

#[test]
fn due_today_is_not_overdue() {
    let item = to_item(
        &issue(Some("2026-11-12"), "Ö1 - Kritik Etki"),
        "https://redmine.example.com",
        day(2026, 11, 12),
    );
    assert!(!item.overdue);
    assert_eq!(item.subtitle.as_deref(), Some("Ö1"));
}

#[test]
fn missing_due_date_has_no_due() {
    let item = to_item(
        &issue(None, "Normal"),
        "https://redmine.example.com",
        day(2026, 11, 12),
    );
    assert_eq!(item.due, None);
    assert!(!item.overdue);
    assert_eq!(item.subtitle.as_deref(), Some("Normal"));
}

#[test]
fn rejected_key_fails_configure() {
    let mut server = mockito::Server::new();
    server
        .mock("GET", "/users/current.json")
        .with_status(401)
        .with_body(json!({"errors": [format!("bad {KEY}")]}).to_string())
        .create();
    let file = keyfile(KEY);
    let err = Redmine::default()
        .configure(config(&server, &file))
        .unwrap_err();
    assert_eq!(err, "Redmine: key rejected");
    assert!(!err.contains(KEY));
}

#[test]
fn empty_key_file_fails_configure() {
    let server = mockito::Server::new();
    let file = keyfile("  \n");
    let err = Redmine::default()
        .configure(config(&server, &file))
        .unwrap_err();
    assert_eq!(
        err,
        format!("Redmine key file is empty: {}", file.path().display())
    );
}

#[test]
fn missing_key_file_fails_configure() {
    let server = mockito::Server::new();
    let cfg = json!({"url": server.url(), "key_file": "/nonexistent/focusdeck-key"});
    let err = Redmine::default().configure(cfg).unwrap_err();
    assert_eq!(
        err,
        "Redmine key file could not be read: /nonexistent/focusdeck-key"
    );
}

#[test]
fn missing_setting_fails_configure() {
    let err = Redmine::default()
        .configure(json!({"url": "http://x"}))
        .unwrap_err();
    assert!(err.starts_with("Invalid settings"), "{err}");
}

#[test]
fn unconfigured_plugin_errors() {
    let err = list(&mut Redmine::default()).unwrap_err();
    assert_eq!(err, "Redmine is not configured");
}

#[test]
fn late_subtitle_by_language() {
    assert_eq!(
        late_subtitle(&i18n::localizer(Some("tr")), 3),
        "3g gecikmiş"
    );
    assert_eq!(late_subtitle(&i18n::localizer(Some("en")), 3), "3d overdue");
}
