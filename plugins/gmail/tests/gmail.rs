use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::{Arc, Mutex};

use chrono::{DateTime, Duration, Local};
use mockito::{Matcher, Mock, ServerGuard};
use seenboard_gmail::client::Message;
use seenboard_gmail::i18n;
use seenboard_gmail::login::{self, LoginArgs};
use seenboard_gmail::oauth;
use seenboard_gmail::plugin::{Gmail, age_label, to_item};
use seenboard_protocol::{ActionParams, ListParams, PROTOCOL_VERSION, Plugin, View};
use serde_json::{Value, json};
use tempfile::{NamedTempFile, TempDir};

const REFRESH: &str = "refresh-secret-1";
const CLIENT_SECRET: &str = "client-secret-1";
const API: &str = "/gmail/v1/users/me";

fn revoked() -> String {
    let bin = std::env::current_exe().unwrap();
    format!("Gmail session expired, run: {} login", bin.display())
}

struct Files {
    client: NamedTempFile,
    token: NamedTempFile,
}

fn files(server: &ServerGuard, key: &str) -> Files {
    let mut client = NamedTempFile::new().unwrap();
    write!(
        client,
        "{}",
        json!({key: {
            "client_id": "cid.example.com",
            "client_secret": CLIENT_SECRET,
            "auth_uri": "https://accounts.example.com/auth",
            "token_uri": format!("{}/token", server.url()),
        }})
    )
    .unwrap();
    let mut token = NamedTempFile::new().unwrap();
    write!(token, "{}", json!({"refresh_token": REFRESH})).unwrap();
    Files { client, token }
}

fn config(server: &ServerGuard, files: &Files) -> Value {
    json!({
        "client_file": files.client.path().to_str().unwrap(),
        "token_file": files.token.path().to_str().unwrap(),
        "api_url": format!("{}{API}//", server.url()),
    })
}

fn token_mock(server: &mut ServerGuard, access: &str, expires_in: u64) -> Mock {
    server
        .mock("POST", "/token")
        .match_body(Matcher::AllOf(vec![
            Matcher::UrlEncoded("grant_type".into(), "refresh_token".into()),
            Matcher::UrlEncoded("refresh_token".into(), REFRESH.into()),
            Matcher::UrlEncoded("client_id".into(), "cid.example.com".into()),
            Matcher::UrlEncoded("client_secret".into(), CLIENT_SECRET.into()),
        ]))
        .with_body(
            json!({"access_token": access, "expires_in": expires_in, "token_type": "Bearer"})
                .to_string(),
        )
        .create()
}

fn configured(server: &mut ServerGuard) -> (Gmail, Files, Mock) {
    let token = token_mock(server, "tok-1", 3600);
    let files = files(server, "installed");
    let mut plugin = Gmail::default();
    plugin.configure(config(server, &files)).unwrap();
    (plugin, files, token)
}

fn list(plugin: &mut Gmail) -> Result<Vec<seenboard_protocol::Item>, String> {
    plugin.list(ListParams {
        view: View::Summary,
    })
}

fn ids_mock(
    server: &mut ServerGuard,
    page_token: Option<&str>,
    ids: &[(&str, &str)],
    next: Option<&str>,
) -> Mock {
    let query = match page_token {
        None => Matcher::Exact("q=is%3Astarred&maxResults=100".into()),
        Some(t) => Matcher::Exact(format!("q=is%3Astarred&maxResults=100&pageToken={t}")),
    };
    let messages: Vec<Value> = ids
        .iter()
        .map(|(id, thread)| json!({"id": id, "threadId": thread}))
        .collect();
    let mut body = json!({"messages": messages, "resultSizeEstimate": messages.len()});
    if let Some(next) = next {
        body["nextPageToken"] = json!(next);
    }
    server
        .mock("GET", format!("{API}/messages").as_str())
        .match_header("authorization", "Bearer tok-1")
        .match_query(query)
        .with_body(body.to_string())
        .create()
}

fn metadata_mock(
    server: &mut ServerGuard,
    id: &str,
    thread: &str,
    date_ms: i64,
    headers: Value,
) -> Mock {
    server
        .mock("GET", format!("{API}/messages/{id}").as_str())
        .match_header("authorization", "Bearer tok-1")
        .match_query(Matcher::Exact(
            "format=metadata&metadataHeaders=From&metadataHeaders=Subject".into(),
        ))
        .with_body(
            json!({
                "id": id, "threadId": thread, "internalDate": date_ms.to_string(),
                "payload": {"headers": headers},
            })
            .to_string(),
        )
        .create()
}

fn headers(from: &str, subject: &str) -> Value {
    json!([{"name": "From", "value": from}, {"name": "Subject", "value": subject}])
}

fn message(from: &str, subject: &str, date: DateTime<Local>) -> Message {
    Message {
        id: "m1".into(),
        thread_id: "t1".into(),
        internal_date_ms: date.timestamp_millis(),
        from: from.into(),
        subject: subject.into(),
    }
}

#[test]
fn describe_matches_contract() {
    let d = Gmail::default().describe();
    assert_eq!(d.name, "gmail");
    assert_eq!(d.version, env!("CARGO_PKG_VERSION"));
    assert_eq!(d.protocol, PROTOCOL_VERSION);
    assert!(d.actions.is_empty());
    assert!(!d.accepts_drop && !d.reminders);
}

#[test]
fn action_has_nothing_to_do() {
    let (mut plugin, _files, _token) = configured(&mut mockito::Server::new());
    let err = plugin
        .action(ActionParams {
            item: "m1".into(),
            action: "archive".into(),
            args: Value::Null,
        })
        .unwrap_err();
    assert_eq!(err, "this box has no actions");
}

#[test]
fn configure_refreshes_access_token() {
    let mut server = mockito::Server::new();
    let token = token_mock(&mut server, "tok-1", 3600).expect(1);
    let files = files(&server, "installed");
    Gmail::default().configure(config(&server, &files)).unwrap();
    token.assert();
}

#[test]
fn web_client_file_is_accepted() {
    let mut server = mockito::Server::new();
    let token = token_mock(&mut server, "tok-1", 3600).expect(1);
    let files = files(&server, "web");
    Gmail::default().configure(config(&server, &files)).unwrap();
    token.assert();
}

#[test]
fn lists_starred_oldest_first_across_pages() {
    let mut server = mockito::Server::new();
    let (mut plugin, _files, _token) = configured(&mut server);
    let first = ids_mock(&mut server, None, &[("m1", "t1"), ("m2", "t2")], Some("p2"));
    let second = ids_mock(&mut server, Some("p2"), &[("m3", "t3")], None);
    let now = Local::now().timestamp_millis();
    let m1 = metadata_mock(
        &mut server,
        "m1",
        "t1",
        now - 1_000,
        headers("A <a@example.com>", "bir"),
    );
    let m2 = metadata_mock(
        &mut server,
        "m2",
        "t2",
        now - 3_000,
        headers("B <b@example.com>", "iki"),
    );
    let m3 = metadata_mock(
        &mut server,
        "m3",
        "t3",
        now - 2_000,
        headers("C <c@example.com>", "üç"),
    );
    let items = list(&mut plugin).unwrap();
    for m in [first, second, m1, m2, m3] {
        m.assert();
    }
    let ids: Vec<_> = items.iter().map(|i| i.id.as_str()).collect();
    assert_eq!(ids, ["m2", "m3", "m1"]);
    assert_eq!(items[0].title, "iki");
    assert_eq!(
        items[0].url.as_deref(),
        Some("https://mail.google.com/mail/#all/t2")
    );
    assert_eq!(items[0].due, None);
}

#[test]
fn empty_starred_list_is_empty() {
    let mut server = mockito::Server::new();
    let (mut plugin, _files, _token) = configured(&mut server);
    server
        .mock("GET", format!("{API}/messages").as_str())
        .match_query(Matcher::Any)
        .with_body(json!({"resultSizeEstimate": 0}).to_string())
        .create();
    assert!(list(&mut plugin).unwrap().is_empty());
}

#[test]
fn missing_headers_fall_back() {
    let mut server = mockito::Server::new();
    let (mut plugin, _files, _token) = configured(&mut server);
    ids_mock(&mut server, None, &[("m1", "t1")], None);
    metadata_mock(
        &mut server,
        "m1",
        "t1",
        Local::now().timestamp_millis(),
        json!([]),
    );
    let items = list(&mut plugin).unwrap();
    assert_eq!(items[0].title, "(no subject)");
    assert_eq!(items[0].subtitle.as_deref(), Some("(no sender) · 0m"));
}

#[test]
fn header_names_compare_case_insensitively() {
    let mut server = mockito::Server::new();
    let (mut plugin, _files, _token) = configured(&mut server);
    ids_mock(&mut server, None, &[("m1", "t1")], None);
    metadata_mock(
        &mut server,
        "m1",
        "t1",
        Local::now().timestamp_millis(),
        json!([{"name": "SUBJECT", "value": "konu"}, {"name": "from", "value": "Ad <a@example.com>"}]),
    );
    let items = list(&mut plugin).unwrap();
    assert_eq!(items[0].title, "konu");
    assert_eq!(items[0].subtitle.as_deref(), Some("Ad · 0m"));
}

#[test]
fn sender_display_forms() {
    let now = Local::now();
    let subtitle = |from: &str| to_item(&message(from, "s", now), now).subtitle.unwrap();
    assert_eq!(subtitle(r#""Ad Soyad" <a@example.com>"#), "Ad Soyad · 0m");
    assert_eq!(subtitle("Ad Soyad <a@example.com>"), "Ad Soyad · 0m");
    assert_eq!(subtitle("a@example.com"), "a@example.com · 0m");
    assert_eq!(subtitle("<a@example.com>"), "a@example.com · 0m");
    assert_eq!(subtitle(r#""" <a@example.com>"#), "a@example.com · 0m");
    assert_eq!(subtitle("  "), "(no sender) · 0m");
}

#[test]
fn age_labels() {
    let now = Local::now();
    let subtitle = |age: Duration| {
        to_item(&message("A", "s", now - age), now)
            .subtitle
            .unwrap()
    };
    assert_eq!(subtitle(Duration::minutes(59)), "A · 59m");
    assert_eq!(subtitle(Duration::minutes(60)), "A · 1h");
    assert_eq!(subtitle(Duration::hours(23)), "A · 23h");
    assert_eq!(subtitle(Duration::hours(24)), "A · 1d");
    assert_eq!(subtitle(Duration::days(5)), "A · 5d");
    assert_eq!(subtitle(Duration::minutes(-5)), "A · 0m");
}

#[test]
fn age_by_language() {
    let tr = i18n::localizer(Some("tr"));
    let en = i18n::localizer(Some("en"));
    assert_eq!(age_label(&tr, Duration::minutes(40)), "40dk");
    assert_eq!(age_label(&tr, Duration::hours(5)), "5s");
    assert_eq!(age_label(&tr, Duration::days(4)), "4g");
    assert_eq!(age_label(&en, Duration::minutes(40)), "40m");
    assert_eq!(age_label(&en, Duration::hours(5)), "5h");
    assert_eq!(age_label(&en, Duration::days(4)), "4d");
}

#[test]
fn control_characters_never_reach_the_terminal() {
    let now = Local::now();
    let item = to_item(
        &message(
            "A\u{1b}[31m <a@example.com>",
            "ko\u{1b}]0;x\u{7}nu\r\n",
            now,
        ),
        now,
    );
    assert_eq!(item.title, "ko]0;xnu");
    assert_eq!(item.subtitle.as_deref(), Some("A[31m · 0m"));
}

#[test]
fn three_day_old_mail_is_overdue() {
    let now = Local::now();
    let young = to_item(&message("A", "s", now - Duration::hours(71)), now);
    assert!(!young.overdue);
    let old = to_item(&message("A", "s", now - Duration::hours(72)), now);
    assert!(old.overdue);
}

#[test]
fn revoked_refresh_token_tells_to_login() {
    for status in [400, 401] {
        let mut server = mockito::Server::new();
        server
            .mock("POST", "/token")
            .with_status(status)
            .with_body(json!({"error": "invalid_grant", "echo": REFRESH}).to_string())
            .create();
        let files = files(&server, "installed");
        let err = Gmail::default()
            .configure(config(&server, &files))
            .unwrap_err();
        assert_eq!(err, revoked());
        assert!(!err.contains(REFRESH) && !err.contains(CLIENT_SECRET));
    }
}

#[test]
fn token_endpoint_failure_names_only_the_status() {
    let mut server = mockito::Server::new();
    server
        .mock("POST", "/token")
        .with_status(503)
        .with_body(REFRESH)
        .create();
    let files = files(&server, "installed");
    let err = Gmail::default()
        .configure(config(&server, &files))
        .unwrap_err();
    assert_eq!(err, "Gmail 503: server error");
}

#[test]
fn access_token_is_renewed_near_expiry() {
    let mut server = mockito::Server::new();
    let token = token_mock(&mut server, "tok-1", 30).expect(3);
    let files = files(&server, "installed");
    let mut plugin = Gmail::default();
    plugin.configure(config(&server, &files)).unwrap();
    server
        .mock("GET", format!("{API}/messages").as_str())
        .match_query(Matcher::Any)
        .with_body(json!({}).to_string())
        .expect(2)
        .create();
    list(&mut plugin).unwrap();
    list(&mut plugin).unwrap();
    token.assert();
}

#[test]
fn valid_access_token_is_reused() {
    let mut server = mockito::Server::new();
    let (mut plugin, _files, token) = configured(&mut server);
    server
        .mock("GET", format!("{API}/messages").as_str())
        .match_query(Matcher::Any)
        .with_body(json!({}).to_string())
        .expect(2)
        .create();
    list(&mut plugin).unwrap();
    list(&mut plugin).unwrap();
    token.expect(1).assert();
}

#[test]
fn http_error_names_the_status_and_reason_only() {
    let cases = [
        (400, "Gmail 400: invalid request"),
        (403, "Gmail 403: no permission"),
        (404, "Gmail 404: not found"),
        (429, "Gmail 429: too many requests"),
        (500, "Gmail 500: server error"),
        (418, "Gmail 418: unexpected response"),
    ];
    for (status, expected) in cases {
        let mut server = mockito::Server::new();
        let (mut plugin, _files, _token) = configured(&mut server);
        server
            .mock("GET", format!("{API}/messages").as_str())
            .match_query(Matcher::Any)
            .with_status(status)
            .with_body(format!("secret body {REFRESH}"))
            .create();
        assert_eq!(list(&mut plugin).unwrap_err(), expected);
    }
}

#[test]
fn api_redirect_is_reported_and_not_followed() {
    let mut server = mockito::Server::new();
    let (mut plugin, _files, _token) = configured(&mut server);
    let elsewhere = server.mock("GET", "/elsewhere").expect(0).create();
    server
        .mock("GET", format!("{API}/messages").as_str())
        .match_query(Matcher::Any)
        .with_status(302)
        .with_header("location", "/elsewhere")
        .create();
    assert_eq!(list(&mut plugin).unwrap_err(), "Gmail 302: redirected");
    elsewhere.assert();
}

#[test]
fn token_endpoint_redirect_does_not_resend_the_secrets() {
    let mut server = mockito::Server::new();
    let elsewhere = server.mock("POST", "/elsewhere").expect(0).create();
    server
        .mock("POST", "/token")
        .with_status(307)
        .with_header("location", "/elsewhere")
        .create();
    let files = files(&server, "installed");
    let err = Gmail::default()
        .configure(config(&server, &files))
        .unwrap_err();
    assert_eq!(err, "Gmail 307: redirected");
    elsewhere.assert();
}

fn client_file(auth_uri: &str, token_uri: &str) -> NamedTempFile {
    let mut file = NamedTempFile::new().unwrap();
    write!(
        file,
        "{}",
        json!({"installed": {
            "client_id": "cid.example.com",
            "client_secret": CLIENT_SECRET,
            "auth_uri": auth_uri,
            "token_uri": token_uri,
        }})
    )
    .unwrap();
    file
}

#[test]
fn plain_http_to_another_machine_is_refused_before_any_request() {
    let server = mockito::Server::new();
    let good = files(&server, "installed");
    let configure = |client: &std::path::Path, api_url: &str| {
        Gmail::default()
            .configure(json!({
                "client_file": client.to_str().unwrap(),
                "token_file": good.token.path().to_str().unwrap(),
                "api_url": api_url,
            }))
            .unwrap_err()
    };
    let api = "https://gmail.googleapis.com/gmail/v1/users/me";
    assert_eq!(
        configure(good.client.path(), "http://gmail.example.com/v1"),
        "Gmail: api_url must use https (plain http only works for localhost)"
    );
    assert_eq!(
        configure(good.client.path(), "gmail.example.com"),
        "Gmail: api_url is invalid"
    );
    let token_uri = client_file("https://a.example.com/auth", "http://a.example.com/token");
    assert_eq!(
        configure(token_uri.path(), api),
        "Gmail: token_uri must use https (plain http only works for localhost)"
    );
    let auth_uri = client_file("http://a.example.com/auth", "https://a.example.com/token");
    assert_eq!(
        configure(auth_uri.path(), api),
        "Gmail: auth_uri must use https (plain http only works for localhost)"
    );
}

fn starred_ids(server: &mut ServerGuard, access: &str, ids: &[&str]) -> Mock {
    let messages: Vec<Value> = ids.iter().map(|id| json!({"id": id})).collect();
    server
        .mock("GET", format!("{API}/messages").as_str())
        .match_header("authorization", format!("Bearer {access}").as_str())
        .match_query(Matcher::Any)
        .with_body(json!({"messages": messages}).to_string())
        .create()
}

#[test]
fn rejected_access_token_is_renewed_and_the_request_repeated() {
    let mut server = mockito::Server::new();
    let (mut plugin, _files, first) = configured(&mut server);
    first.remove();
    let renewed = token_mock(&mut server, "tok-2", 3600).expect(1);
    let stale = server
        .mock("GET", format!("{API}/messages").as_str())
        .match_header("authorization", "Bearer tok-1")
        .match_query(Matcher::Any)
        .with_status(401)
        .expect(1)
        .create();
    let fresh = starred_ids(&mut server, "tok-2", &[]).expect(1);
    assert!(list(&mut plugin).unwrap().is_empty());
    for m in [renewed, stale, fresh] {
        m.assert();
    }
}

#[test]
fn access_token_rejected_twice_is_an_error_not_a_loop() {
    let mut server = mockito::Server::new();
    let (mut plugin, _files, first) = configured(&mut server);
    first.remove();
    let renewed = token_mock(&mut server, "tok-2", 3600).expect(1);
    let rejected = server
        .mock("GET", format!("{API}/messages").as_str())
        .match_query(Matcher::Any)
        .with_status(401)
        .expect(2)
        .create();
    assert_eq!(
        list(&mut plugin).unwrap_err(),
        "Gmail 401: sign-in rejected"
    );
    renewed.assert();
    rejected.assert();
}

#[test]
fn absurd_token_lifetime_does_not_crash() {
    let mut server = mockito::Server::new();
    let _token = token_mock(&mut server, "tok-1", u64::MAX);
    let files = files(&server, "installed");
    let mut plugin = Gmail::default();
    plugin.configure(config(&server, &files)).unwrap();
    starred_ids(&mut server, "tok-1", &[]);
    assert!(list(&mut plugin).unwrap().is_empty());
}

#[test]
fn metadata_is_fetched_once_per_starred_message() {
    let mut server = mockito::Server::new();
    let (mut plugin, _files, _token) = configured(&mut server);
    let now = Local::now().timestamp_millis();
    let both = starred_ids(&mut server, "tok-1", &["m1", "m2"]);
    let m1 = metadata_mock(&mut server, "m1", "t1", now - 2_000, headers("A", "bir")).expect(1);
    let m2 = metadata_mock(&mut server, "m2", "t2", now - 1_000, headers("B", "iki")).expect(1);
    let titles = |plugin: &mut Gmail| -> Vec<String> {
        list(plugin).unwrap().into_iter().map(|i| i.title).collect()
    };
    assert_eq!(titles(&mut plugin), ["bir", "iki"]);
    assert_eq!(titles(&mut plugin), ["bir", "iki"]);
    m1.assert();
    m2.assert();

    both.remove();
    starred_ids(&mut server, "tok-1", &["m2", "m3"]);
    let m3 = metadata_mock(&mut server, "m3", "t3", now, headers("C", "üç")).expect(1);
    assert_eq!(titles(&mut plugin), ["iki", "üç"]);
    m2.assert();
    m3.assert();
}

#[test]
fn endless_page_tokens_are_cut_off() {
    let mut server = mockito::Server::new();
    let (mut plugin, _files, _token) = configured(&mut server);
    let counter = std::sync::atomic::AtomicUsize::new(0);
    let endless = server
        .mock("GET", format!("{API}/messages").as_str())
        .match_query(Matcher::Any)
        .with_body_from_request(move |_| {
            let next = counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            json!({"messages": [], "nextPageToken": format!("p{next}")})
                .to_string()
                .into_bytes()
        })
        .expect(100)
        .create();
    assert!(list(&mut plugin).unwrap().is_empty());
    endless.assert();
}

#[test]
fn empty_token_file_fails_configure() {
    let server = mockito::Server::new();
    let files = files(&server, "installed");
    std::fs::write(files.token.path(), r#"{"refresh_token":" "}"#).unwrap();
    let err = Gmail::default()
        .configure(config(&server, &files))
        .unwrap_err();
    assert_eq!(
        err,
        format!("Gmail key file is empty: {}", files.token.path().display())
    );
}

#[test]
fn missing_setting_fails_configure() {
    let err = Gmail::default()
        .configure(json!({"client_file": "/c"}))
        .unwrap_err();
    assert!(err.starts_with("Invalid settings"), "{err}");
}

#[test]
fn unconfigured_plugin_errors() {
    assert_eq!(
        list(&mut Gmail::default()).unwrap_err(),
        "Gmail is not configured"
    );
}

fn get(port: u16, target: &str) -> String {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    write!(stream, "GET {target} HTTP/1.1\r\nHost: localhost\r\n\r\n").unwrap();
    let mut out = String::new();
    stream.read_to_string(&mut out).unwrap();
    out
}

#[test]
fn login_exchanges_code_and_writes_token() {
    let mut server = mockito::Server::new();
    let files = files(&server, "installed");
    let dir = TempDir::new().unwrap();
    let token_path = dir.path().join("gmail/token.json");
    let verifier = Arc::new(Mutex::new(String::new()));
    let seen = verifier.clone();
    let exchange = server
        .mock("POST", "/token")
        .match_request(move |req| {
            let body = req.utf8_lossy_body().unwrap();
            let pairs: Vec<(String, String)> = reqwest::Url::parse(&format!("http://x/?{body}"))
                .unwrap()
                .query_pairs()
                .map(|(k, v)| (k.into_owned(), v.into_owned()))
                .collect();
            let field = |name: &str| {
                pairs
                    .iter()
                    .find(|(k, _)| k == name)
                    .map(|(_, v)| v.clone())
                    .unwrap_or_default()
            };
            *seen.lock().unwrap() = field("code_verifier");
            field("grant_type") == "authorization_code"
                && field("code") == "the-code"
                && field("client_id") == "cid.example.com"
                && field("client_secret") == CLIENT_SECRET
                && field("redirect_uri").starts_with("http://127.0.0.1:")
        })
        .with_body(
            json!({"access_token": "a", "expires_in": 3600, "refresh_token": "new-refresh"})
                .to_string(),
        )
        .create();

    let args = LoginArgs {
        client_file: files.client.path().to_path_buf(),
        token_file: token_path.clone(),
        hint: Some("me@example.com".into()),
    };
    let (tx, rx) = std::sync::mpsc::channel();
    let challenge = Arc::new(Mutex::new(String::new()));
    let announced = challenge.clone();
    login::run_with(args, move |url| {
        let url = reqwest::Url::parse(url).unwrap();
        let q: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();
        assert_eq!(
            url.as_str().split('?').next(),
            Some("https://accounts.example.com/auth")
        );
        assert_eq!(q["client_id"], "cid.example.com");
        assert_eq!(q["response_type"], "code");
        assert_eq!(q["scope"], "https://www.googleapis.com/auth/gmail.readonly");
        assert_eq!(q["access_type"], "offline");
        assert_eq!(q["prompt"], "consent");
        assert_eq!(q["code_challenge_method"], "S256");
        assert_eq!(q["login_hint"], "me@example.com");
        *announced.lock().unwrap() = q["code_challenge"].clone();
        let redirect = reqwest::Url::parse(&q["redirect_uri"]).unwrap();
        assert_eq!(redirect.host_str(), Some("127.0.0.1"));
        assert_eq!(redirect.path(), "/");
        let port = redirect.port().unwrap();
        let state = q["state"].clone();
        std::thread::spawn(move || {
            get(port, "/favicon.ico");
            tx.send(get(port, &format!("/?code=the-code&state={state}")))
                .unwrap();
        });
    })
    .unwrap();

    exchange.assert();
    let page = rx.recv().unwrap();
    assert!(page.ends_with("seenboard: Gmail connected, you can close this tab."));
    assert_eq!(
        oauth::pkce_challenge(&verifier.lock().unwrap()),
        *challenge.lock().unwrap()
    );
    assert_eq!(
        std::fs::read_to_string(&token_path).unwrap(),
        r#"{"refresh_token":"new-refresh"}"#
    );
}

#[test]
fn login_without_refresh_token_fails() {
    let mut server = mockito::Server::new();
    let files = files(&server, "installed");
    let dir = TempDir::new().unwrap();
    let token_path = dir.path().join("token.json");
    server
        .mock("POST", "/token")
        .with_body(json!({"access_token": "a", "expires_in": 3600}).to_string())
        .create();
    let args = LoginArgs {
        client_file: files.client.path().to_path_buf(),
        token_file: token_path.clone(),
        hint: None,
    };
    let err = login::run_with(args, |url| {
        let url = reqwest::Url::parse(url).unwrap();
        let q: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();
        assert!(!q.contains_key("login_hint"));
        let port = reqwest::Url::parse(&q["redirect_uri"])
            .unwrap()
            .port()
            .unwrap();
        let state = q["state"].clone();
        std::thread::spawn(move || get(port, &format!("/?code=c&state={state}")));
    })
    .unwrap_err();
    assert_eq!(err, "Google did not return a refresh token");
    assert!(!token_path.exists());
}

#[test]
fn login_ignores_a_forged_callback_and_finishes_with_the_real_one() {
    let mut server = mockito::Server::new();
    let files = files(&server, "installed");
    let dir = TempDir::new().unwrap();
    let token_path = dir.path().join("token.json");
    let exchange = server
        .mock("POST", "/token")
        .match_body(Matcher::UrlEncoded("code".into(), "real".into()))
        .with_body(
            json!({"access_token": "a", "expires_in": 3600, "refresh_token": "new-refresh"})
                .to_string(),
        )
        .expect(1)
        .create();
    let args = LoginArgs {
        client_file: files.client.path().to_path_buf(),
        token_file: token_path.clone(),
        hint: None,
    };
    let (tx, rx) = std::sync::mpsc::channel();
    login::run_with(args, move |url| {
        let url = reqwest::Url::parse(url).unwrap();
        let q: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();
        let port = reqwest::Url::parse(&q["redirect_uri"])
            .unwrap()
            .port()
            .unwrap();
        let state = q["state"].clone();
        std::thread::spawn(move || {
            tx.send(get(port, "/?code=forged&state=forged")).unwrap();
            tx.send(get(port, "/?error=access_denied")).unwrap();
            get(port, &format!("/?code=real&state={state}"));
        });
    })
    .unwrap();
    exchange.assert();
    for _ in 0..2 {
        let page = rx.recv().unwrap();
        assert!(page.starts_with("HTTP/1.1 400"), "{page}");
    }
    assert!(token_path.exists());
}

#[test]
fn login_refuses_a_web_client_before_opening_the_browser() {
    let server = mockito::Server::new();
    let files = files(&server, "web");
    let dir = TempDir::new().unwrap();
    let args = LoginArgs {
        client_file: files.client.path().to_path_buf(),
        token_file: dir.path().join("token.json"),
        hint: None,
    };
    let err = login::run_with(args, |_| panic!("the browser must not be opened")).unwrap_err();
    assert_eq!(
        err,
        "Login needs a Desktop app OAuth client; this client file is for a web application"
    );
}
