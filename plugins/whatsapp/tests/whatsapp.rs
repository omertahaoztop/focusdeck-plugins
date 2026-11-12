use std::io::Write;
use std::os::unix::fs::PermissionsExt;

use chrono::{
    DateTime, FixedOffset, Local, LocalResult, NaiveDate, NaiveDateTime, TimeDelta, TimeZone, Utc,
};
use focusdeck_protocol::{ActionParams, Item, ListParams, PROTOCOL_VERSION, Plugin, View};
use focusdeck_whatsapp::client::Message;
use focusdeck_whatsapp::i18n;
use focusdeck_whatsapp::plugin::{WhatsApp, parse_since_in, to_item_in};
use mockito::{Matcher, Mock, ServerGuard};
use serde_json::{Value, json};
use tempfile::NamedTempFile;

const KEY: &str = "s3cret-key";
const USER: &str = "me";
const BASIC: &str = "Basic bWU6czNjcmV0LWtleQ==";
const JID: &str = "15550001111@s.whatsapp.net";
const CHAT_PATH: &str = "/chat/15550001111%40s.whatsapp.net/messages";

fn keyfile(content: &str) -> NamedTempFile {
    let mut file = NamedTempFile::new().unwrap();
    write!(file, "{content}").unwrap();
    std::fs::set_permissions(file.path(), std::fs::Permissions::from_mode(0o600)).unwrap();
    file
}

fn config(server: &ServerGuard, keyfile: &NamedTempFile) -> Value {
    json!({
        "url": server.url(),
        "user": USER,
        "key_file": keyfile.path().to_str().unwrap(),
        "jid": JID,
    })
}

struct Fixture {
    server: ServerGuard,
    plugin: WhatsApp,
    _key: NamedTempFile,
}

impl Fixture {
    fn list(&mut self) -> Result<Vec<Item>, String> {
        self.plugin.list(ListParams {
            view: View::Summary,
        })
    }

    fn done(&mut self, id: &str) -> Result<Option<Item>, String> {
        self.plugin.action(ActionParams {
            item: id.into(),
            action: "done".into(),
            args: Value::Null,
        })
    }
}

fn probe(server: &mut ServerGuard, expect: usize) -> Mock {
    server
        .mock("GET", CHAT_PATH)
        .match_header("authorization", BASIC)
        .match_query(Matcher::AllOf(vec![
            Matcher::UrlEncoded("limit".into(), "1".into()),
            Matcher::UrlEncoded("offset".into(), "0".into()),
        ]))
        .with_body(page(vec![], 0).to_string())
        .expect(expect)
        .create()
}

fn fixture_with(extra: Value) -> Fixture {
    let mut server = mockito::Server::new();
    probe(&mut server, 1);
    let key = keyfile(&format!("{KEY}\n"));
    let mut plugin = WhatsApp::default();
    let mut cfg = config(&server, &key);
    cfg.as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());
    plugin.configure(cfg).unwrap();
    Fixture {
        server,
        plugin,
        _key: key,
    }
}

fn fixture() -> Fixture {
    fixture_with(json!({}))
}

fn page(data: Vec<Value>, total: usize) -> Value {
    json!({
        "code": "SUCCESS", "message": "ok",
        "results": {
            "data": data,
            "pagination": {"limit": 100, "offset": 0, "total": total},
            "chat_info": {"jid": JID},
        }
    })
}

fn msg(id: &str, content: &str, timestamp: &str) -> Value {
    json!({
        "id": id, "content": content, "timestamp": timestamp,
        "is_from_me": true, "media_type": null, "filename": null,
    })
}

fn media(id: &str, kind: &str, filename: Option<&str>) -> Value {
    json!({
        "id": id, "content": "", "timestamp": "2026-11-10T10:00:00Z",
        "is_from_me": true, "media_type": kind, "filename": filename,
    })
}

fn listing(server: &mut ServerGuard, offset: usize, data: Vec<Value>, total: usize) -> Mock {
    server
        .mock("GET", CHAT_PATH)
        .match_header("authorization", BASIC)
        .match_header(
            "user-agent",
            Matcher::Regex(r"^focusdeck-whatsapp/\d+\.\d+\.\d+".into()),
        )
        .match_query(Matcher::AllOf(vec![
            Matcher::UrlEncoded("limit".into(), "100".into()),
            Matcher::UrlEncoded("offset".into(), offset.to_string()),
        ]))
        .with_body(page(data, total).to_string())
        .create()
}

fn titles(items: &[Item]) -> Vec<&str> {
    items.iter().map(|i| i.title.as_str()).collect()
}

fn message(value: Value) -> Message {
    serde_json::from_value(value).unwrap()
}

fn now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 11, 12, 12, 0, 0).unwrap()
}

fn bare() -> WhatsApp {
    WhatsApp::default()
}

fn item_of(value: Value) -> Item {
    to_item_in(&i18n::localizer(Some("en")), &message(value), now())
}

#[test]
fn describe_matches_contract() {
    let d = bare().describe();
    assert_eq!(d.name, "whatsapp");
    assert_eq!(d.version, "0.2.0");
    assert_eq!(d.protocol, PROTOCOL_VERSION);
    assert_eq!(d.actions, ["done"]);
    assert!(!d.accepts_drop && !d.reminders);
}

#[test]
fn lists_self_chat_newest_first() {
    let mut f = fixture();
    let m = f
        .server
        .mock("GET", format!("{CHAT_PATH}?limit=100&offset=0").as_str())
        .match_header("authorization", BASIC)
        .with_body(
            page(
                vec![
                    msg("a", "eski", "2026-11-01T10:00:00Z"),
                    msg("b", "yeni", "2026-11-02T10:00:00Z"),
                ],
                2,
            )
            .to_string(),
        )
        .create();
    let items = f.list().unwrap();
    m.assert();
    let ids: Vec<_> = items.iter().map(|i| i.id.as_str()).collect();
    assert_eq!(ids, ["b", "a"]);
    assert_eq!(titles(&items), ["yeni", "eski"]);
    assert_eq!(items[0].due, None);
    assert!(!items[0].overdue);
}

#[test]
fn pages_until_total() {
    let mut f = fixture();
    let first: Vec<Value> = (0..100)
        .map(|i| msg(&format!("m{i}"), "x", "2026-11-01T10:00:00Z"))
        .collect();
    let second: Vec<Value> = (100..150)
        .map(|i| msg(&format!("m{i}"), "x", "2026-11-01T10:00:00Z"))
        .collect();
    let a = listing(&mut f.server, 0, first, 150);
    let b = listing(&mut f.server, 100, second, 150);
    let items = f.list().unwrap();
    a.assert();
    b.assert();
    assert_eq!(items.len(), 150);
}

#[test]
fn incomplete_listing_is_an_error() {
    let mut f = fixture();
    listing(&mut f.server, 0, full_page(0), 150);
    listing(&mut f.server, 100, vec![], 150);
    let err = f.list().unwrap_err();
    assert_eq!(err, "WhatsApp: listing came back incomplete (100/150)");
}

#[test]
fn done_deletes_the_message_from_the_chat() {
    let mut f = fixture();
    let m = deletion(&mut f.server, "/message/b/delete");
    assert_eq!(f.done("b").unwrap(), None);
    m.assert();
}

fn deletion(server: &mut ServerGuard, path: &str) -> Mock {
    server
        .mock("POST", path)
        .match_header("authorization", BASIC)
        .match_body(Matcher::Json(json!({"phone": JID})))
        .with_body(json!({"code": "SUCCESS", "message": "ok", "results": null}).to_string())
        .expect(1)
        .create()
}

#[test]
fn failed_delete_is_an_error() {
    let mut f = fixture();
    f.server
        .mock("POST", "/message/b/delete")
        .with_status(500)
        .with_body(format!("secret body {KEY}"))
        .create();
    assert_eq!(f.done("b").unwrap_err(), "WhatsApp 500: server error");
}

#[test]
fn message_id_stays_one_path_segment() {
    let mut f = fixture();
    let m = deletion(&mut f.server, "/message/a%2F..%2Fb/delete");
    f.done("a/../b").unwrap();
    m.assert();
}

#[test]
fn id_that_cannot_be_a_path_segment_is_rejected_unsent() {
    for id in ["", ".", ".."] {
        let mut f = fixture();
        let stray = f.server.mock("POST", Matcher::Any).expect(0).create();
        assert_eq!(f.done(id).unwrap_err(), "WhatsApp: message id is invalid");
        stray.assert();
    }
}

#[test]
fn only_reads_the_chat_and_deletes_the_done_message() {
    let mut server = mockito::Server::new();
    let mut strays: Vec<Mock> = ["PUT", "PATCH", "DELETE"]
        .iter()
        .map(|method| {
            server
                .mock(method, Matcher::Any)
                .with_status(500)
                .expect(0)
                .create()
        })
        .collect();
    strays.push(
        server
            .mock("POST", Matcher::Any)
            .match_request(|r| r.path() != "/message/a/delete")
            .with_status(500)
            .expect(0)
            .create(),
    );
    strays.push(
        server
            .mock("GET", Matcher::Any)
            .match_request(|r| !r.path().starts_with(CHAT_PATH))
            .with_status(500)
            .expect(0)
            .create(),
    );
    let probe = probe(&mut server, 1);
    let key = keyfile(KEY);
    let mut plugin = WhatsApp::default();
    plugin.configure(config(&server, &key)).unwrap();
    let m = server
        .mock("GET", CHAT_PATH)
        .match_query(Matcher::UrlEncoded("limit".into(), "100".into()))
        .with_body(page(vec![msg("a", "x", "2026-11-01T10:00:00Z")], 1).to_string())
        .expect(1)
        .create();
    let d = deletion(&mut server, "/message/a/delete");
    plugin
        .list(ListParams {
            view: View::Summary,
        })
        .unwrap();
    plugin
        .action(ActionParams {
            item: "a".into(),
            action: "done".into(),
            args: Value::Null,
        })
        .unwrap();
    probe.assert();
    m.assert();
    d.assert();
    for stray in strays {
        stray.assert();
    }
}

#[test]
fn since_is_sent_as_start_time() {
    let mut f = fixture_with(json!({"since": "2026-11-01"}));
    let start = NaiveDate::from_ymd_opt(2026, 11, 1)
        .unwrap()
        .and_hms_opt(0, 0, 0)
        .unwrap()
        .and_local_timezone(Local)
        .earliest()
        .unwrap()
        .with_timezone(&Utc)
        .to_rfc3339();
    let m = f
        .server
        .mock("GET", CHAT_PATH)
        .match_query(Matcher::AllOf(vec![
            Matcher::UrlEncoded("limit".into(), "100".into()),
            Matcher::UrlEncoded("offset".into(), "0".into()),
            Matcher::UrlEncoded("start_time".into(), start),
        ]))
        .with_body(page(vec![msg("a", "x", "2026-11-02T10:00:00Z")], 1).to_string())
        .create();
    assert_eq!(titles(&f.list().unwrap()), ["x"]);
    m.assert();
}

#[test]
fn invalid_since_fails_configure() {
    let server = mockito::Server::new();
    let key = keyfile(KEY);
    let mut cfg = config(&server, &key);
    cfg["since"] = json!("01.11.2026");
    let err = bare().configure(cfg).unwrap_err();
    assert_eq!(err, "WhatsApp: since is invalid, use YYYY-MM-DD");
}

#[test]
fn media_without_text_gets_placeholder_title() {
    let cases = [
        (media("1", "image", None), "(image)"),
        (media("2", "video", None), "(video)"),
        (media("3", "audio", None), "(audio)"),
        (media("4", "document", Some("ad.pdf")), "(file: ad.pdf)"),
        (media("5", "document", None), "(file)"),
        (media("6", "sticker", None), "(media)"),
        (msg("7", "  \n", "2026-11-10T10:00:00Z"), "(empty message)"),
    ];
    for (value, title) in cases {
        assert_eq!(item_of(value).title, title);
    }
    let tr = i18n::localizer(Some("tr"));
    let item = to_item_in(&tr, &message(media("1", "image", None)), now());
    assert_eq!(item.title, "(görsel)");
    let item = to_item_in(&tr, &message(media("4", "document", Some("ad.pdf"))), now());
    assert_eq!(item.title, "(dosya: ad.pdf)");
}

#[test]
fn multiline_content_goes_to_description() {
    let item = item_of(msg("1", "\nilk satır\n\nikinci\n", "2026-11-10T10:00:00Z"));
    assert_eq!(item.title, "ilk satır");
    assert_eq!(item.description.as_deref(), Some("ilk satır\n\nikinci"));
    let single = item_of(msg("2", "tek satır\n", "2026-11-10T10:00:00Z"));
    assert_eq!(single.description, None);
}

#[test]
fn first_link_becomes_url() {
    let item = item_of(msg(
        "1",
        "bak https://example.com/a?x=1, sonra http://example.org/b",
        "2026-11-10T10:00:00Z",
    ));
    assert_eq!(item.url.as_deref(), Some("https://example.com/a?x=1"));
    assert_eq!(
        item_of(msg("2", "link yok", "2026-11-10T10:00:00Z")).url,
        None
    );
}

#[test]
fn age_labels_follow_the_language() {
    let en = item_of(msg("1", "x", "2026-11-12T11:30:00Z"));
    assert_eq!(en.subtitle.as_deref(), Some("30m"));
    let hours = item_of(msg("2", "x", "2026-11-12T07:00:00Z"));
    assert_eq!(hours.subtitle.as_deref(), Some("5h"));
    let tr = i18n::localizer(Some("tr"));
    let old = message(msg("3", "x", "2026-11-09T12:00:00Z"));
    assert_eq!(to_item_in(&tr, &old, now()).subtitle.as_deref(), Some("3g"));
    let future = item_of(msg("4", "x", "2026-11-13T12:00:00Z"));
    assert_eq!(future.subtitle.as_deref(), Some("0m"));
}

#[test]
fn hostile_characters_are_stripped_from_shown_text() {
    let item = item_of(msg(
        "1",
        "\u{1b}[2J\n\u{202e}merhaba\u{7}",
        "2026-11-10T10:00:00Z",
    ));
    assert_eq!(item.title, "[2J");
    assert_eq!(item.description.as_deref(), Some("[2J\nmerhaba"));
}

#[test]
fn call_logs_are_skipped() {
    let mut f = fixture();
    listing(
        &mut f.server,
        0,
        vec![
            media("c", "call", None),
            msg("a", "gerçek", "2026-11-01T10:00:00Z"),
        ],
        2,
    );
    assert_eq!(titles(&f.list().unwrap()), ["gerçek"]);
}

#[test]
fn null_fields_are_tolerated() {
    let m = message(json!({
        "id": "1", "content": null, "timestamp": "2026-11-10T10:00:00Z",
        "is_from_me": false, "media_type": null, "filename": null, "extra": 1,
    }));
    assert_eq!(m.content, "");
}

#[test]
fn rejected_password_fails_configure() {
    let mut server = mockito::Server::new();
    server
        .mock("GET", CHAT_PATH)
        .match_query(Matcher::Any)
        .with_status(401)
        .with_body(format!("bad {KEY}"))
        .create();
    let key = keyfile(KEY);
    let err = bare().configure(config(&server, &key)).unwrap_err();
    assert_eq!(err, "WhatsApp: password rejected");
    assert!(!err.contains(KEY));
}

#[test]
fn not_found_address_fails_configure() {
    let mut server = mockito::Server::new();
    server
        .mock("GET", CHAT_PATH)
        .match_query(Matcher::Any)
        .with_status(404)
        .create();
    let key = keyfile(KEY);
    let err = bare().configure(config(&server, &key)).unwrap_err();
    assert_eq!(err, "WhatsApp: address not found, check url");
}

#[test]
fn group_jid_is_rejected() {
    let server = mockito::Server::new();
    let key = keyfile(KEY);
    for jid in [
        "120363000000000000@g.us",
        "15550001111",
        "@s.whatsapp.net",
        "a/b@s.whatsapp.net",
        "a@s.whatsapp.net/x",
        "a b@lid",
    ] {
        let mut cfg = config(&server, &key);
        cfg["jid"] = json!(jid);
        let err = bare().configure(cfg).unwrap_err();
        assert_eq!(err, "WhatsApp: jid is invalid", "{jid}");
    }
}

#[test]
fn lid_jid_is_accepted() {
    let mut server = mockito::Server::new();
    let m = server
        .mock("GET", "/chat/12345%40lid/messages")
        .match_query(Matcher::Any)
        .with_body(page(vec![], 0).to_string())
        .create();
    let key = keyfile(KEY);
    let mut cfg = config(&server, &key);
    cfg["jid"] = json!("12345@lid");
    bare().configure(cfg).unwrap();
    m.assert();
}

#[test]
fn empty_key_file_fails_configure() {
    let server = mockito::Server::new();
    let key = keyfile("  \n");
    let err = bare().configure(config(&server, &key)).unwrap_err();
    assert_eq!(
        err,
        format!("WhatsApp key file is empty: {}", key.path().display())
    );
}

#[test]
fn plain_http_to_another_machine_is_refused() {
    let key = keyfile(KEY);
    let mut cfg = json!({"url": "http://gowa.example.com", "user": USER, "jid": JID});
    cfg["key_file"] = json!(key.path().to_str().unwrap());
    let err = bare().configure(cfg).unwrap_err();
    assert_eq!(
        err,
        "WhatsApp: url must use https (plain http only works for localhost)"
    );
}

#[test]
fn http_error_names_the_status_and_reason_only() {
    let cases = [
        (403, "WhatsApp 403: no permission"),
        (429, "WhatsApp 429: too many requests"),
        (500, "WhatsApp 500: server error"),
        (418, "WhatsApp 418: unexpected response"),
    ];
    for (status, expected) in cases {
        let mut f = fixture();
        f.server
            .mock("GET", CHAT_PATH)
            .match_query(Matcher::UrlEncoded("limit".into(), "100".into()))
            .with_status(status)
            .with_body(format!("secret body {KEY}"))
            .create();
        assert_eq!(f.list().unwrap_err(), expected);
    }
}

#[test]
fn missing_setting_fails_configure() {
    let err = bare()
        .configure(json!({"url": "https://x.example.com", "user": "u", "key_file": "/k"}))
        .unwrap_err();
    assert!(err.starts_with("Invalid settings"), "{err}");
}

#[test]
fn unconfigured_plugin_errors() {
    let mut plugin = bare();
    let err = plugin
        .list(ListParams {
            view: View::Summary,
        })
        .unwrap_err();
    assert_eq!(err, "WhatsApp is not configured");
}

#[test]
fn unknown_action_is_error() {
    let mut f = fixture();
    let err = f
        .plugin
        .action(ActionParams {
            item: "a".into(),
            action: "delete".into(),
            args: Value::Null,
        })
        .unwrap_err();
    assert_eq!(err, "Unknown action: delete");
}

fn full_page(from: usize) -> Vec<Value> {
    (from..from + 100)
        .map(|i| msg(&format!("m{i}"), "x", "2026-11-01T10:00:00Z"))
        .collect()
}

#[test]
fn since_ignores_total() {
    let mut f = fixture_with(json!({"since": "2026-11-05"}));
    let m = f
        .server
        .mock("GET", CHAT_PATH)
        .match_query(Matcher::AllOf(vec![
            Matcher::UrlEncoded("offset".into(), "0".into()),
            Matcher::Regex("start_time=2026-11-0[45]T".into()),
        ]))
        .with_body(page(vec![msg("c", "x", "2026-11-06T10:00:00Z")], 500).to_string())
        .create();
    assert_eq!(titles(&f.list().unwrap()), ["x"]);
    m.assert();
}

#[test]
fn since_is_resolved_in_the_given_zone() {
    let zone = FixedOffset::east_opt(3 * 3600).unwrap();
    let start = parse_since_in("2026-11-05", &zone).unwrap();
    assert_eq!(start.to_rfc3339(), "2026-11-04T21:00:00+00:00");
}

#[derive(Clone)]
struct SkipsMidnight(NaiveDate);

impl TimeZone for SkipsMidnight {
    type Offset = FixedOffset;

    fn from_offset(_: &FixedOffset) -> Self {
        unreachable!()
    }

    fn offset_from_local_date(&self, _: &NaiveDate) -> LocalResult<FixedOffset> {
        unreachable!()
    }

    fn offset_from_local_datetime(&self, local: &NaiveDateTime) -> LocalResult<FixedOffset> {
        let gap = self.0.and_hms_opt(0, 0, 0).unwrap();
        let east = |hours| FixedOffset::east_opt(hours * 3600).unwrap();
        if *local < gap {
            LocalResult::Single(east(2))
        } else if *local < gap + TimeDelta::hours(1) {
            LocalResult::None
        } else {
            LocalResult::Single(east(3))
        }
    }

    fn offset_from_utc_date(&self, _: &NaiveDate) -> FixedOffset {
        unreachable!()
    }

    fn offset_from_utc_datetime(&self, _: &NaiveDateTime) -> FixedOffset {
        unreachable!()
    }
}

#[test]
fn since_on_a_day_without_midnight_starts_at_the_first_existing_hour() {
    let day = NaiveDate::from_ymd_opt(2026, 4, 24).unwrap();
    let start = parse_since_in("2026-04-24", &SkipsMidnight(day)).unwrap();
    assert_eq!(start.to_rfc3339(), "2026-04-23T22:00:00+00:00");
}

#[test]
fn repeated_id_across_pages_is_an_error() {
    let mut f = fixture();
    let mut second = full_page(100);
    second[0] = msg("m0", "x", "2026-11-01T10:00:00Z");
    listing(&mut f.server, 0, full_page(0), 200);
    listing(&mut f.server, 100, second, 200);
    assert!(f.list().is_err());
}

#[test]
fn running_out_of_pages_is_an_error() {
    let mut f = fixture();
    for i in 0..20 {
        listing(&mut f.server, i * 100, full_page(i * 100), 2500);
    }
    listing(&mut f.server, 2000, full_page(2000), 2500);
    assert!(f.list().is_err());
}

#[test]
fn rows_with_zero_total_are_an_error() {
    let mut f = fixture();
    listing(
        &mut f.server,
        0,
        vec![msg("a", "x", "2026-11-01T10:00:00Z")],
        0,
    );
    assert!(f.list().is_err());
}

fn exactly_cap(f: &mut Fixture, total: usize) {
    for i in 0..20 {
        listing(&mut f.server, i * 100, full_page(i * 100), total);
    }
    listing(&mut f.server, 2000, vec![], total);
}

#[test]
fn exactly_two_thousand_rows_is_complete() {
    let mut f = fixture();
    exactly_cap(&mut f, 2000);
    assert_eq!(f.list().unwrap().len(), 2000);
}

#[test]
fn exactly_two_thousand_rows_is_complete_with_since() {
    let mut f = fixture_with(json!({"since": "2026-11-05"}));
    for i in 0..20 {
        f.server
            .mock("GET", CHAT_PATH)
            .match_query(Matcher::AllOf(vec![
                Matcher::UrlEncoded("offset".into(), (i * 100).to_string()),
                Matcher::Regex("start_time=".into()),
            ]))
            .with_body(page(full_page(i * 100), 9000).to_string())
            .create();
    }
    f.server
        .mock("GET", CHAT_PATH)
        .match_query(Matcher::AllOf(vec![
            Matcher::UrlEncoded("offset".into(), "2000".into()),
            Matcher::Regex("start_time=".into()),
        ]))
        .with_body(page(vec![], 9000).to_string())
        .create();
    assert_eq!(f.list().unwrap().len(), 2000);
}

#[test]
fn row_past_the_cap_is_an_error_with_since() {
    let mut f = fixture_with(json!({"since": "2026-11-05"}));
    for i in 0..21 {
        f.server
            .mock("GET", CHAT_PATH)
            .match_query(Matcher::AllOf(vec![
                Matcher::UrlEncoded("offset".into(), (i * 100).to_string()),
                Matcher::Regex("start_time=".into()),
            ]))
            .with_body(page(full_page(i * 100), 9000).to_string())
            .create();
    }
    assert!(f.list().is_err());
}
