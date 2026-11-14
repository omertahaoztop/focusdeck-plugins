use std::io::Write;
use std::os::unix::fs::PermissionsExt;

use mockito::{Matcher, Mock, ServerGuard};
use seenboard_protocol::{ActionParams, ListParams, PROTOCOL_VERSION, Plugin, View};
use seenboard_zulip::client::Message;
use seenboard_zulip::i18n;
use seenboard_zulip::plugin::{Zulip, to_item, to_item_in};
use serde_json::{Value, json};
use tempfile::NamedTempFile;

const KEY: &str = "s3cret-key";
const EMAIL: &str = "me@example.com";
const BASIC: &str = "Basic bWVAZXhhbXBsZS5jb206czNjcmV0LWtleQ==";

fn keyfile(content: &str) -> NamedTempFile {
    let mut file = NamedTempFile::new().unwrap();
    write!(file, "{content}").unwrap();
    std::fs::set_permissions(file.path(), std::fs::Permissions::from_mode(0o600)).unwrap();
    file
}

fn config(server: &ServerGuard, keyfile: &NamedTempFile) -> Value {
    json!({
        "url": server.url(),
        "email": EMAIL,
        "key_file": keyfile.path().to_str().unwrap(),
    })
}

fn me(server: &mut ServerGuard) -> Mock {
    server
        .mock("GET", "/api/v1/users/me")
        .match_header("authorization", BASIC)
        .with_body(json!({"result": "success"}).to_string())
        .create()
}

fn configured(server: &mut ServerGuard) -> (Zulip, NamedTempFile, Mock) {
    let user = me(server);
    let file = keyfile(&format!("{KEY}\n"));
    let mut plugin = Zulip::default();
    plugin.configure(config(server, &file)).unwrap();
    (plugin, file, user)
}

fn stream_msg(id: u64, content: &str) -> Value {
    json!({
        "id": id, "content": content, "sender_full_name": "Ayşe Yılmaz", "type": "stream",
        "display_recipient": "genel", "subject": "duyurular",
    })
}

fn private_msg(id: u64, content: &str) -> Value {
    json!({
        "id": id, "content": content, "sender_full_name": "Ali Veli", "type": "private",
        "display_recipient": [{"id": 1, "email": "a@example.com"}, {"id": 2, "email": "b@example.org"}],
        "subject": "",
    })
}

fn messages(
    server: &mut ServerGuard,
    anchor: &str,
    include_anchor: Option<&str>,
    msgs: Vec<Value>,
    found_oldest: bool,
) -> Mock {
    let mut query = vec![
        Matcher::UrlEncoded("anchor".into(), anchor.into()),
        Matcher::UrlEncoded("num_before".into(), "1000".into()),
        Matcher::UrlEncoded("num_after".into(), "0".into()),
        Matcher::UrlEncoded("apply_markdown".into(), "false".into()),
        Matcher::UrlEncoded(
            "narrow".into(),
            r#"[{"operator":"is","operand":"starred"}]"#.into(),
        ),
    ];
    if let Some(value) = include_anchor {
        query.push(Matcher::UrlEncoded("include_anchor".into(), value.into()));
    }
    server
        .mock("GET", "/api/v1/messages")
        .match_header("authorization", BASIC)
        .match_header(
            "user-agent",
            Matcher::Regex(r"^seenboard-zulip/\d+\.\d+\.\d+".into()),
        )
        .match_query(Matcher::AllOf(query))
        .with_body(
            json!({
                "result": "success", "messages": msgs,
                "found_oldest": found_oldest, "found_newest": true,
            })
            .to_string(),
        )
        .create()
}

fn list(plugin: &mut Zulip) -> Result<Vec<seenboard_protocol::Item>, String> {
    plugin.list(ListParams {
        view: View::Summary,
    })
}

fn message(value: Value) -> Message {
    serde_json::from_value(value).unwrap()
}

#[test]
fn describe_matches_contract() {
    let d = Zulip::default().describe();
    assert_eq!(d.name, "zulip");
    assert_eq!(d.version, env!("CARGO_PKG_VERSION"));
    assert_eq!(d.protocol, PROTOCOL_VERSION);
    assert_eq!(d.actions, ["unstar"]);
    assert!(!d.accepts_drop && !d.reminders);
}

#[test]
fn lists_starred_newest_first() {
    let mut server = mockito::Server::new();
    let (mut plugin, _file, _me) = configured(&mut server);
    let m = messages(
        &mut server,
        "newest",
        None,
        vec![stream_msg(5, "eski"), private_msg(9, "yeni")],
        true,
    );
    let items = list(&mut plugin).unwrap();
    m.assert();
    let ids: Vec<_> = items.iter().map(|i| i.id.as_str()).collect();
    assert_eq!(ids, ["9", "5"]);
    assert_eq!(items[0].title, "yeni");
    assert_eq!(items[0].subtitle.as_deref(), Some("Ali Veli · private"));
    assert_eq!(
        items[1].subtitle.as_deref(),
        Some("Ayşe Yılmaz · #genel > duyurular")
    );
    assert_eq!(items[1].url, Some(format!("{}/#narrow/id/5", server.url())));
    assert_eq!(items[1].due, None);
    assert!(!items[1].overdue);
}

#[test]
fn follows_pages_until_found_oldest() {
    let mut server = mockito::Server::new();
    let (mut plugin, _file, _me) = configured(&mut server);
    let first = messages(
        &mut server,
        "newest",
        None,
        vec![stream_msg(30, "c"), stream_msg(20, "b")],
        false,
    );
    let second = messages(
        &mut server,
        "20",
        Some("false"),
        vec![stream_msg(10, "a")],
        true,
    );
    let items = list(&mut plugin).unwrap();
    first.assert();
    second.assert();
    let ids: Vec<_> = items.iter().map(|i| i.id.as_str()).collect();
    assert_eq!(ids, ["30", "20", "10"]);
}

#[test]
fn placeholders_by_language() {
    let base = "https://zulip.example.com";
    let tr = i18n::localizer(Some("tr"));
    let en = i18n::localizer(Some("en"));
    let blank = message(stream_msg(1, "\n  \n"));
    assert_eq!(to_item_in(&tr, &blank, base).title, "(boş mesaj)");
    assert_eq!(to_item_in(&en, &blank, base).title, "(empty message)");
    let private = message(private_msg(2, "x"));
    let subtitle = |localizer| to_item_in(localizer, &private, base).subtitle;
    assert_eq!(subtitle(&tr).as_deref(), Some("Ali Veli · özel"));
    assert_eq!(subtitle(&en).as_deref(), Some("Ali Veli · private"));
}

#[test]
fn blank_content_gets_placeholder_title() {
    let base = "https://zulip.example.com";
    let blank = to_item(&message(stream_msg(1, "\n  \n")), base);
    assert_eq!(blank.title, "(empty message)");
    let quoted = to_item(&message(stream_msg(2, "> alıntı\nasıl metin")), base);
    assert_eq!(quoted.title, "alıntı");
    let bare_quote = to_item(&message(stream_msg(3, ">\n>> \nmetin")), base);
    assert_eq!(bare_quote.title, "metin");
}

#[test]
fn unstar_posts_remove_flag() {
    let mut server = mockito::Server::new();
    let (mut plugin, _file, _me) = configured(&mut server);
    let m = server
        .mock("POST", "/api/v1/messages/flags")
        .match_header("authorization", BASIC)
        .match_header(
            "user-agent",
            Matcher::Regex(r"^seenboard-zulip/\d+\.\d+\.\d+".into()),
        )
        .match_body(Matcher::AllOf(vec![
            Matcher::UrlEncoded("messages".into(), "[42]".into()),
            Matcher::UrlEncoded("op".into(), "remove".into()),
            Matcher::UrlEncoded("flag".into(), "starred".into()),
        ]))
        .with_body(json!({"result": "success"}).to_string())
        .create();
    let out = plugin
        .action(ActionParams {
            item: "42".into(),
            action: "unstar".into(),
            args: Value::Null,
        })
        .unwrap();
    m.assert();
    assert!(out.is_none());
}

#[test]
fn unknown_action_is_error() {
    let mut server = mockito::Server::new();
    let (mut plugin, _file, _me) = configured(&mut server);
    let err = plugin
        .action(ActionParams {
            item: "42".into(),
            action: "delete".into(),
            args: Value::Null,
        })
        .unwrap_err();
    assert_eq!(err, "Unknown action: delete");
}

#[test]
fn http_error_names_the_status_and_reason_only() {
    let cases = [
        (400, "Zulip 400: invalid request"),
        (403, "Zulip 403: no permission"),
        (404, "Zulip 404: not found"),
        (429, "Zulip 429: too many requests"),
        (500, "Zulip 500: server error"),
        (418, "Zulip 418: unexpected response"),
    ];
    for (status, expected) in cases {
        let mut server = mockito::Server::new();
        let (mut plugin, _file, _me) = configured(&mut server);
        server
            .mock("GET", "/api/v1/messages")
            .match_query(Matcher::Any)
            .with_status(status)
            .with_body(format!("secret body {KEY}"))
            .create();
        assert_eq!(list(&mut plugin).unwrap_err(), expected);
    }
}

#[test]
fn redirect_is_reported_and_not_followed() {
    let mut server = mockito::Server::new();
    let (mut plugin, _file, _me) = configured(&mut server);
    let elsewhere = server.mock("GET", "/elsewhere").expect(0).create();
    server
        .mock("GET", "/api/v1/messages")
        .match_query(Matcher::Any)
        .with_status(302)
        .with_header("location", "/elsewhere")
        .create();
    assert_eq!(
        list(&mut plugin).unwrap_err(),
        "Zulip 302: redirected, check the url"
    );
    elsewhere.assert();
}

#[test]
fn plain_http_to_another_machine_is_refused_before_any_request() {
    let file = keyfile(KEY);
    let configure = |url: &str| {
        Zulip::default()
            .configure(json!({
                "url": url, "email": EMAIL, "key_file": file.path().to_str().unwrap(),
            }))
            .unwrap_err()
    };
    assert_eq!(
        configure("http://zulip.example.com"),
        "Zulip: url must use https (plain http only works for localhost)"
    );
    assert_eq!(configure("zulip.example.com"), "Zulip: url is invalid");
}

#[test]
fn hostile_characters_are_stripped_from_shown_text() {
    let hostile = message(json!({
        "id": 1, "content": "\u{1b}[2J\n> \u{202e}merhaba\u{7}", "type": "stream",
        "sender_full_name": "Ay\u{1b}şe", "display_recipient": "ge\u{202e}nel",
        "subject": "duyuru\u{9b}lar",
    }));
    let item = to_item(&hostile, "https://zulip.example.com");
    assert_eq!(item.title, "[2J");
    assert_eq!(item.subtitle.as_deref(), Some("Ayşe · #genel > duyurular"));
    let private = message(json!({
        "id": 2, "content": "\u{202e}", "type": "private",
        "sender_full_name": "A\u{2066}li", "display_recipient": [], "subject": "",
    }));
    let item = to_item(&private, "https://zulip.example.com");
    assert_eq!(item.title, "(empty message)");
    assert_eq!(item.subtitle.as_deref(), Some("Ali · private"));
}

#[test]
fn a_server_that_never_reports_the_oldest_message_is_cut_off() {
    let mut server = mockito::Server::new();
    let (mut plugin, _file, _me) = configured(&mut server);
    let endless = server
        .mock("GET", "/api/v1/messages")
        .match_query(Matcher::Any)
        .with_body_from_request(|request| {
            let url =
                reqwest::Url::parse(&format!("http://x{}", request.path_and_query())).unwrap();
            let anchor = url
                .query_pairs()
                .find(|(key, _)| key == "anchor")
                .map(|(_, value)| value.into_owned())
                .unwrap();
            let id = anchor.parse::<u64>().map_or(1_000_000, |a| a - 1);
            json!({"result": "success", "messages": [stream_msg(id, "x")], "found_oldest": false})
                .to_string()
                .into_bytes()
        })
        .expect(100)
        .create();
    assert_eq!(list(&mut plugin).unwrap().len(), 100);
    endless.assert();
}

#[test]
fn rejected_key_fails_configure() {
    let mut server = mockito::Server::new();
    server
        .mock("GET", "/api/v1/users/me")
        .with_status(401)
        .with_body(json!({"result": "error", "msg": format!("bad {KEY}")}).to_string())
        .create();
    let file = keyfile(KEY);
    let err = Zulip::default()
        .configure(config(&server, &file))
        .unwrap_err();
    assert_eq!(err, "Zulip 401: key rejected");
    assert!(!err.contains(KEY));
}

#[test]
fn empty_key_file_fails_configure() {
    let server = mockito::Server::new();
    let file = keyfile("  \n");
    let err = Zulip::default()
        .configure(config(&server, &file))
        .unwrap_err();
    assert_eq!(
        err,
        format!("Zulip key file is empty: {}", file.path().display())
    );
}

#[test]
fn missing_setting_fails_configure() {
    let err = Zulip::default()
        .configure(json!({"url": "http://x", "key_file": "/k"}))
        .unwrap_err();
    assert!(err.starts_with("Invalid settings"), "{err}");
}

#[test]
fn unconfigured_plugin_errors() {
    let err = list(&mut Zulip::default()).unwrap_err();
    assert_eq!(err, "Zulip is not configured");
}
