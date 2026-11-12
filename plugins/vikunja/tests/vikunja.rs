use std::io::Write;
use std::os::unix::fs::PermissionsExt;

use focusdeck_protocol::{
    ActionParams, DropParams, Item, Link, ListParams, PROTOCOL_VERSION, Plugin, RemindersParams,
    Source, View,
};
use focusdeck_vikunja::plugin::Vikunja;
use mockito::{Matcher, Mock, ServerGuard};
use serde_json::{Value, json};
use tempfile::NamedTempFile;

const TOKEN: &str = "s3cret-token";

struct Ctx {
    server: ServerGuard,
    plugin: Vikunja,
    _keyfile: NamedTempFile,
    _base: Vec<Mock>,
}

fn keyfile_new() -> NamedTempFile {
    let mut file = NamedTempFile::new().unwrap();
    writeln!(file, "{TOKEN}").unwrap();
    std::fs::set_permissions(file.path(), std::fs::Permissions::from_mode(0o600)).unwrap();
    file
}

fn config(
    server: &ServerGuard,
    keyfile: &NamedTempFile,
    project: &str,
    bucket: Option<&str>,
) -> Value {
    let mut cfg = json!({
        "url": server.url(),
        "token_file": keyfile.path().to_str().unwrap(),
        "project": project,
    });
    if let Some(bucket) = bucket {
        cfg["bucket"] = json!(bucket);
    }
    cfg
}

fn mock(server: &mut ServerGuard, method: &str, path: &str) -> Mock {
    server
        .mock(method, path)
        .match_header("authorization", format!("Bearer {TOKEN}").as_str())
}

fn kanban_views() -> Value {
    json!([
        {"id": 36, "view_kind": "list"},
        {"id": 37, "view_kind": "kanban", "default_bucket_id": 59, "done_bucket_id": 61},
    ])
}

fn base_mocks(server: &mut ServerGuard, views: Value) -> Vec<Mock> {
    vec![
        mock(server, "GET", "/api/v1/projects?per_page=200&page=1")
            .match_header("user-agent", Matcher::Regex("^focusdeck-vikunja/".into()))
            .with_body(json!([{"id": 3, "title": "Başka"}, {"id": 9, "title": "Home"}]).to_string())
            .create(),
        mock(server, "GET", "/api/v1/projects?per_page=200&page=2")
            .with_body("[]")
            .create(),
        mock(server, "GET", "/api/v1/projects/9/views")
            .with_body(views.to_string())
            .create(),
        mock(server, "GET", "/api/v1/projects/9/views/37/buckets")
            .with_body(
                json!([
                    {"id": 59, "title": "Yeni"},
                    {"id": 62, "title": "Karşı tarafı bekliyorum"},
                    {"id": 61, "title": "Bitti"},
                ])
                .to_string(),
            )
            .create(),
    ]
}

fn ctx(bucket: Option<&str>) -> Ctx {
    let mut server = mockito::Server::new();
    let base = base_mocks(&mut server, kanban_views());
    let keyfile = keyfile_new();
    let mut plugin = Vikunja::default();
    plugin
        .configure(config(&server, &keyfile, "Home", bucket))
        .unwrap();
    Ctx {
        server,
        plugin,
        _keyfile: keyfile,
        _base: base,
    }
}

fn task(id: u64, title: &str, position: f64) -> Value {
    json!({
        "id": id, "title": title, "description": "", "done": false,
        "due_date": "0001-01-01T00:00:00Z", "position": position,
        "bucket_id": 59, "project_id": 9, "reminders": null, "labels": null,
    })
}

fn view_tasks(server: &mut ServerGuard, columns: Value) -> Mock {
    mock(
        server,
        "GET",
        "/api/v1/projects/9/views/37/tasks?per_page=200&page=1",
    )
    .with_body(columns.to_string())
    .create()
}

fn list(c: &mut Ctx) -> Vec<Item> {
    c.plugin
        .list(ListParams {
            view: View::Summary,
        })
        .unwrap()
}

fn action(id: &str, name: &str, args: Value) -> ActionParams {
    ActionParams {
        item: id.into(),
        action: name.into(),
        args,
    }
}

fn note(title: &str) -> Item {
    Item {
        id: String::new(),
        title: title.into(),
        subtitle: None,
        url: None,
        due: None,
        overdue: false,
        source: None,
        description: None,
        labels: vec![],
        from: None,
        links: vec![],
    }
}

fn created_task() -> String {
    task(100, "yeni", 1.0).to_string()
}

fn position_columns() -> Value {
    json!([{"id": 59, "title": "Yeni", "tasks": [
        task(1, "bir", 100.0), task(2, "iki", 200.0), task(3, "üç", 300.0),
    ]}])
}

#[test]
fn describe_matches_contract() {
    let d = Vikunja::default().describe();
    assert_eq!(d.name, "vikunja");
    assert_eq!(d.protocol, PROTOCOL_VERSION);
    assert_eq!(d.actions, ["done", "delete", "move", "reorder"]);
    assert!(d.accepts_drop && d.reminders);
}

#[test]
fn configure_resolves_project_view_and_bucket() {
    let mut c = ctx(Some("Karşı tarafı bekliyorum"));
    let m = view_tasks(
        &mut c.server,
        json!([
            {"id": 59, "title": "Yeni", "tasks": [task(1, "a", 1.0)]},
            {"id": 62, "title": "Karşı tarafı bekliyorum", "tasks": [task(2, "b", 2.0)]},
        ]),
    );
    let items = list(&mut c);
    m.assert();
    assert_eq!(
        items.iter().map(|i| i.id.as_str()).collect::<Vec<_>>(),
        ["2"]
    );
}

#[test]
fn configure_matches_project_case_insensitively() {
    let mut server = mockito::Server::new();
    let _base = base_mocks(&mut server, kanban_views());
    let keyfile = keyfile_new();
    let mut plugin = Vikunja::default();
    plugin
        .configure(config(&server, &keyfile, "home", None))
        .unwrap();
}

#[test]
fn configure_accepts_numeric_view_kind() {
    let mut server = mockito::Server::new();
    let _base = base_mocks(
        &mut server,
        json!([{"id": 37, "view_kind": 3, "default_bucket_id": 59, "done_bucket_id": 61}]),
    );
    let keyfile = keyfile_new();
    Vikunja::default()
        .configure(config(&server, &keyfile, "Home", None))
        .unwrap();
}

#[test]
fn configure_fails_without_kanban_view() {
    let mut server = mockito::Server::new();
    let _base = base_mocks(&mut server, json!([{"id": 36, "view_kind": "list"}]));
    let keyfile = keyfile_new();
    let err = Vikunja::default()
        .configure(config(&server, &keyfile, "Home", None))
        .unwrap_err();
    assert_eq!(err, "Project has no kanban view: Home");
}

#[test]
fn configure_unknown_project() {
    let mut server = mockito::Server::new();
    let _base = base_mocks(&mut server, kanban_views());
    let keyfile = keyfile_new();
    let err = Vikunja::default()
        .configure(config(&server, &keyfile, "Missing", None))
        .unwrap_err();
    assert_eq!(err, "No such project in Vikunja: Missing");
}

#[test]
fn configure_unknown_bucket() {
    let mut server = mockito::Server::new();
    let _base = base_mocks(&mut server, kanban_views());
    let keyfile = keyfile_new();
    let err = Vikunja::default()
        .configure(config(&server, &keyfile, "Home", Some("Ghost")))
        .unwrap_err();
    assert_eq!(err, "No such bucket in Vikunja: Ghost");
}

#[test]
fn configure_unauthorized_hides_token() {
    let mut server = mockito::Server::new();
    let _m = server
        .mock("GET", "/api/v1/projects?per_page=200&page=1")
        .with_status(401)
        .with_body(json!({"message": format!("bad {TOKEN}")}).to_string())
        .create();
    let keyfile = keyfile_new();
    let err = Vikunja::default()
        .configure(config(&server, &keyfile, "Home", None))
        .unwrap_err();
    assert_eq!(err, "Vikunja 401: invalid token");
    assert!(!err.contains(TOKEN));
}

#[test]
fn configure_reports_missing_token_file() {
    let server = mockito::Server::new();
    let cfg = json!({"url": server.url(), "token_file": "/nonexistent/focusdeck-token", "project": "Home"});
    let err = Vikunja::default().configure(cfg).unwrap_err();
    assert!(err.starts_with("Could not read token file"), "{err}");
}

#[test]
fn configure_reports_missing_setting() {
    let err = Vikunja::default()
        .configure(json!({"url": "http://x"}))
        .unwrap_err();
    assert!(err.starts_with("Invalid settings"), "{err}");
}

#[test]
fn unconfigured_plugin_errors() {
    let err = Vikunja::default()
        .list(ListParams {
            view: View::Summary,
        })
        .unwrap_err();
    assert_eq!(err, "Vikunja is not configured");
}

#[test]
fn list_hides_done_bucket_and_done_tasks() {
    let mut c = ctx(None);
    let mut finished = task(4, "bitmiş", 4.0);
    finished["done"] = json!(true);
    view_tasks(
        &mut c.server,
        json!([
            {"id": 59, "title": "Yeni", "tasks": [task(1, "açık", 1.0), finished]},
            {"id": 61, "title": "Bitti", "tasks": [task(3, "kapalı", 3.0)]},
        ]),
    );
    let items = list(&mut c);
    assert_eq!(
        items.iter().map(|i| i.id.as_str()).collect::<Vec<_>>(),
        ["1"]
    );
}

#[test]
fn list_due_handling() {
    let mut c = ctx(None);
    let mut past = task(2, "geçmiş", 2.0);
    past["due_date"] = json!("2020-01-02T10:00:00Z");
    let mut future = task(3, "gelecek", 3.0);
    future["due_date"] = json!("2999-01-02T10:00:00Z");
    view_tasks(
        &mut c.server,
        json!([{"id": 59, "title": "Yeni", "tasks": [task(1, "tarihsiz", 1.0), past, future]}]),
    );
    let items = list(&mut c);
    assert_eq!((items[0].due, items[0].overdue), (None, false));
    assert!(items[1].due.is_some() && items[1].overdue);
    assert!(items[2].due.is_some() && !items[2].overdue);
}

#[test]
fn list_reads_source_from_description() {
    let mut c = ctx(None);
    let mut t = task(1, "bağlı", 1.0);
    t["description"] = json!("<p>not</p><p>focusdeck-source: redmine:12345</p>");
    view_tasks(
        &mut c.server,
        json!([{"id": 59, "title": "Yeni", "tasks": [t, task(2, "serbest", 2.0)]}]),
    );
    let items = list(&mut c);
    assert_eq!(
        items[0].source,
        Some(Source {
            plugin: "redmine".into(),
            id: "12345".into()
        })
    );
    assert_eq!(items[1].source, None);
}

#[test]
fn list_turns_from_label_into_subtitle() {
    let mut c = ctx(None);
    let mut t = task(7, "ara", 1.0);
    t["labels"] = json!([{"id": 1, "title": "kimden:ali"}, {"id": 2, "title": "mail"}]);
    view_tasks(
        &mut c.server,
        json!([{"id": 59, "title": "Yeni", "tasks": [t]}]),
    );
    let items = list(&mut c);
    assert!(items[0].subtitle.as_deref().unwrap().starts_with("@ali"));
    assert_eq!(items[0].labels, ["mail"]);
    assert_eq!(items[0].from.as_deref(), Some("ali"));
    assert_eq!(items[0].url, Some(format!("{}/tasks/7", c.server.url())));
}

#[test]
fn list_orders_by_position_across_columns() {
    let mut c = ctx(None);
    view_tasks(
        &mut c.server,
        json!([
            {"id": 59, "title": "Yeni", "tasks": [task(1, "a", 300.0)]},
            {"id": 62, "title": "Karşı tarafı bekliyorum", "tasks": [task(2, "b", 100.0)]},
        ]),
    );
    let items = list(&mut c);
    assert_eq!(
        items.iter().map(|i| i.id.as_str()).collect::<Vec<_>>(),
        ["2", "1"]
    );
}

#[test]
fn action_done_moves_to_done_bucket() {
    let mut c = ctx(None);
    let m = mock(
        &mut c.server,
        "POST",
        "/api/v1/projects/9/views/37/buckets/61/tasks",
    )
    .match_body(Matcher::PartialJson(json!({"task_id": 5})))
    .with_body("{}")
    .create();
    assert_eq!(
        c.plugin.action(action("5", "done", Value::Null)).unwrap(),
        None
    );
    m.assert();
}

#[test]
fn action_delete_removes_task() {
    let mut c = ctx(None);
    let m = mock(&mut c.server, "DELETE", "/api/v1/tasks/5")
        .with_body(json!({"message": "ok"}).to_string())
        .create();
    assert_eq!(
        c.plugin.action(action("5", "delete", Value::Null)).unwrap(),
        None
    );
    m.assert();
}

#[test]
fn action_rejects_non_numeric_id_and_unknown_action() {
    let mut c = ctx(None);
    assert!(
        c.plugin
            .action(action("../9", "delete", Value::Null))
            .is_err()
    );
    let err = c
        .plugin
        .action(action("5", "boom", Value::Null))
        .unwrap_err();
    assert_eq!(err, "Unknown action: boom");
}

#[test]
fn action_move_to_named_bucket() {
    let mut c = ctx(None);
    let m = mock(
        &mut c.server,
        "POST",
        "/api/v1/projects/9/views/37/buckets/62/tasks",
    )
    .match_body(Matcher::Json(
        json!({"task_id": 5, "bucket_id": 62, "project_view_id": 37}),
    ))
    .with_body("{}")
    .create();
    let args = json!({"bucket": "Karşı tarafı bekliyorum"});
    assert_eq!(c.plugin.action(action("5", "move", args)).unwrap(), None);
    m.assert();
}

fn reorder(c: &mut Ctx, id: &str, position: u64, expected: f64) {
    let m = mock(
        &mut c.server,
        "POST",
        &format!("/api/v1/tasks/{id}/position"),
    )
    .match_body(Matcher::Json(json!({
        "task_id": id.parse::<u64>().unwrap(), "project_view_id": 37, "position": expected,
    })))
    .with_body("{}")
    .create();
    let result = c
        .plugin
        .action(action(id, "reorder", json!({"position": position})));
    assert_eq!(result.unwrap(), None);
    m.assert();
}

#[test]
fn reorder_between_neighbours() {
    let mut c = ctx(None);
    view_tasks(&mut c.server, position_columns());
    list(&mut c);
    reorder(&mut c, "1", 1, 250.0);
}

#[test]
fn reorder_to_top_halves_first_position() {
    let mut c = ctx(None);
    view_tasks(&mut c.server, position_columns());
    list(&mut c);
    reorder(&mut c, "3", 0, 50.0);
}

#[test]
fn reorder_to_bottom_adds_step() {
    let mut c = ctx(None);
    view_tasks(&mut c.server, position_columns());
    list(&mut c);
    reorder(&mut c, "1", 2, 65836.0);
    // position past the end is clamped to the bottom
    reorder(&mut c, "2", 99, 65836.0 + 65536.0);
}

#[test]
fn reorder_lists_first_when_nothing_cached() {
    let mut c = ctx(None);
    let m = view_tasks(&mut c.server, position_columns());
    reorder(&mut c, "1", 1, 250.0);
    m.assert();
}

#[test]
fn reorder_uses_updated_positions_for_next_move() {
    let mut c = ctx(None);
    view_tasks(&mut c.server, position_columns());
    list(&mut c);
    reorder(&mut c, "3", 0, 50.0);
    // list is now 3(50), 1(100), 2(200); moving 2 to the top halves 50
    reorder(&mut c, "2", 0, 25.0);
}

#[test]
fn reorder_unknown_task_errors() {
    let mut c = ctx(None);
    view_tasks(&mut c.server, position_columns());
    let err = c
        .plugin
        .action(action("42", "reorder", json!({"position": 0})))
        .unwrap_err();
    assert_eq!(err, "Task is not in the list: 42");
}

fn label_search(c: &mut Ctx, title: &str, found: Value) -> Mock {
    mock(&mut c.server, "GET", "/api/v1/labels")
        .match_query(Matcher::UrlEncoded("s".into(), title.into()))
        .with_body(found.to_string())
        .create()
}

#[test]
fn drop_creates_task_with_labels_links_and_reminder() {
    let mut c = ctx(None);
    // search is a substring match, so a longer title must not be taken for the label
    label_search(&mut c, "mail", json!([{"id": 4, "title": "mailing"}]));
    label_search(
        &mut c,
        "kimden:ali",
        json!([{"id": 7, "title": "kimden:ali"}]),
    );
    let create_label = mock(&mut c.server, "PUT", "/api/v1/labels")
        .match_body(Matcher::Json(json!({"title": "mail"})))
        .with_body(json!({"id": 5, "title": "mail"}).to_string())
        .create();
    let due = "2026-11-13T09:00:00+03:00";
    let create = mock(&mut c.server, "PUT", "/api/v1/projects/9/tasks")
        .match_body(Matcher::Json(json!({
            "title": "Ali'yi ara",
            "description": "<p>ayrıntı</p><p><a href=\"https://redmine.example.com/issues/12345\">R12345</a></p>",
            "due_date": due,
            "reminders": [{"reminder": due}],
        })))
        .with_body(created_task())
        .create();
    let bulk = mock(&mut c.server, "POST", "/api/v1/tasks/100/labels/bulk")
        .match_body(Matcher::Json(json!({"labels": [{"id": 5}, {"id": 7}]})))
        .with_body("{}")
        .create();
    let mut item = note("Ali'yi ara");
    item.due = Some(chrono::DateTime::parse_from_rfc3339(due).unwrap());
    item.labels = vec!["mail".into()];
    item.from = Some("ali".into());
    item.description = Some("ayrıntı".into());
    item.links = vec![Link {
        label: "R12345".into(),
        url: "https://redmine.example.com/issues/12345".into(),
    }];
    let created = c.plugin.drop_item(DropParams { item, from: None }).unwrap();
    create_label.assert();
    create.assert();
    bulk.assert();
    assert_eq!(created.id, "100");
    assert_eq!(created.labels, ["mail"]);
    assert_eq!(created.from.as_deref(), Some("ali"));
}

#[test]
fn drop_without_extras_sends_title_only() {
    let mut c = ctx(None);
    let create = mock(&mut c.server, "PUT", "/api/v1/projects/9/tasks")
        .match_body(Matcher::Json(json!({"title": "yalın", "description": ""})))
        .with_body(created_task())
        .create();
    c.plugin
        .drop_item(DropParams {
            item: note("yalın"),
            from: None,
        })
        .unwrap();
    create.assert();
}

#[test]
fn drop_escapes_description_html() {
    let mut c = ctx(None);
    let create = mock(&mut c.server, "PUT", "/api/v1/projects/9/tasks")
        .match_body(Matcher::PartialJson(
            json!({"description": "<p>&lt;b&gt;</p>"}),
        ))
        .with_body(created_task())
        .create();
    let mut item = note("html");
    item.description = Some("<b>".into());
    c.plugin.drop_item(DropParams { item, from: None }).unwrap();
    create.assert();
}

#[test]
fn drop_writes_item_source_into_description() {
    let mut c = ctx(None);
    let create = mock(&mut c.server, "PUT", "/api/v1/projects/9/tasks")
        .match_body(Matcher::PartialJson(
            json!({"description": "<p>focusdeck-source: redmine:12345</p>"}),
        ))
        .with_body(created_task())
        .create();
    let mut item = note("S3");
    item.source = Some(Source {
        plugin: "redmine".into(),
        id: "12345".into(),
    });
    c.plugin.drop_item(DropParams { item, from: None }).unwrap();
    create.assert();
}

#[test]
fn drop_uses_from_as_source_when_item_has_none() {
    let mut c = ctx(None);
    let create = mock(&mut c.server, "PUT", "/api/v1/projects/9/tasks")
        .match_body(Matcher::PartialJson(
            json!({"description": "<p>focusdeck-source: redmine:12345</p>"}),
        ))
        .with_body(created_task())
        .create();
    let from = Source {
        plugin: "redmine".into(),
        id: "12345".into(),
    };
    c.plugin
        .drop_item(DropParams {
            item: note("S3"),
            from: Some(from),
        })
        .unwrap();
    create.assert();
}

#[test]
fn drop_moves_new_task_into_configured_bucket() {
    let mut c = ctx(Some("Karşı tarafı bekliyorum"));
    let _create = mock(&mut c.server, "PUT", "/api/v1/projects/9/tasks")
        .with_body(created_task())
        .create();
    let moved = mock(
        &mut c.server,
        "POST",
        "/api/v1/projects/9/views/37/buckets/62/tasks",
    )
    .match_body(Matcher::Json(
        json!({"task_id": 100, "bucket_id": 62, "project_view_id": 37}),
    ))
    .with_body("{}")
    .create();
    c.plugin
        .drop_item(DropParams {
            item: note("yeni"),
            from: None,
        })
        .unwrap();
    moved.assert();
}

fn vikunja_origin() -> Option<Source> {
    Some(Source {
        plugin: "vikunja".into(),
        id: "77".into(),
    })
}

#[test]
fn drop_from_same_project_moves_instead_of_creating() {
    let mut c = ctx(Some("Karşı tarafı bekliyorum"));
    let create = mock(&mut c.server, "PUT", "/api/v1/projects/9/tasks")
        .expect(0)
        .create();
    let _get = mock(&mut c.server, "GET", "/api/v1/tasks/77")
        .with_body(task(77, "taşınan", 1.0).to_string())
        .create();
    let moved = mock(
        &mut c.server,
        "POST",
        "/api/v1/projects/9/views/37/buckets/62/tasks",
    )
    .match_body(Matcher::Json(
        json!({"task_id": 77, "bucket_id": 62, "project_view_id": 37}),
    ))
    .with_body("{}")
    .create();
    let item = c
        .plugin
        .drop_item(DropParams {
            item: note("taşınan"),
            from: vikunja_origin(),
        })
        .unwrap();
    create.assert();
    moved.assert();
    assert_eq!(item.id, "77");
}

#[test]
fn drop_from_same_project_without_bucket_uses_default_bucket() {
    let mut c = ctx(None);
    let _get = mock(&mut c.server, "GET", "/api/v1/tasks/77")
        .with_body(task(77, "taşınan", 1.0).to_string())
        .create();
    let moved = mock(
        &mut c.server,
        "POST",
        "/api/v1/projects/9/views/37/buckets/59/tasks",
    )
    .match_body(Matcher::Json(
        json!({"task_id": 77, "bucket_id": 59, "project_view_id": 37}),
    ))
    .with_body("{}")
    .create();
    c.plugin
        .drop_item(DropParams {
            item: note("taşınan"),
            from: vikunja_origin(),
        })
        .unwrap();
    moved.assert();
}

#[test]
fn drop_from_other_project_sends_full_object_with_new_project() {
    let mut c = ctx(None);
    let original = json!({
        "id": 77, "title": "taşınan", "description": "<p>not</p>", "done": false,
        "due_date": "2026-11-13T09:00:00Z", "position": 12.5, "bucket_id": 4, "project_id": 5,
        "priority": 3, "percent_done": 0.5, "hex_color": "ff0000",
        "reminders": [{"reminder": "2026-11-13T08:00:00Z"}],
        "labels": [{"id": 7, "title": "kimden:ali"}],
        "assignees": [{"id": 2, "username": "ali"}],
    });
    let mut expected = original.clone();
    expected["project_id"] = json!(9);
    let _get = mock(&mut c.server, "GET", "/api/v1/tasks/77")
        .with_body(original.to_string())
        .create();
    let update = mock(&mut c.server, "POST", "/api/v1/tasks/77")
        .match_body(Matcher::Json(expected.clone()))
        .with_body(expected.to_string())
        .create();
    let item = c
        .plugin
        .drop_item(DropParams {
            item: note("taşınan"),
            from: vikunja_origin(),
        })
        .unwrap();
    update.assert();
    assert_eq!(item.id, "77");
    assert_eq!(item.from.as_deref(), Some("ali"));
}

#[test]
fn drop_from_other_project_moves_into_configured_bucket() {
    let mut c = ctx(Some("Karşı tarafı bekliyorum"));
    let mut other = task(77, "taşınan", 1.0);
    other["project_id"] = json!(5);
    let mut moved_task = other.clone();
    moved_task["project_id"] = json!(9);
    let _get = mock(&mut c.server, "GET", "/api/v1/tasks/77")
        .with_body(other.to_string())
        .create();
    let _update = mock(&mut c.server, "POST", "/api/v1/tasks/77")
        .with_body(moved_task.to_string())
        .create();
    let bucket = mock(
        &mut c.server,
        "POST",
        "/api/v1/projects/9/views/37/buckets/62/tasks",
    )
    .with_body("{}")
    .create();
    c.plugin
        .drop_item(DropParams {
            item: note("taşınan"),
            from: vikunja_origin(),
        })
        .unwrap();
    bucket.assert();
}

#[test]
fn reminders_return_tasks_reminded_after_since() {
    let mut c = ctx(None);
    let mut late = task(1, "geç", 1.0);
    late["reminders"] = json!([{"reminder": "2026-01-05T09:30:00Z"}]);
    let mut early = task(2, "erken", 2.0);
    early["reminders"] = json!([{"reminder": "2026-01-05T08:00:00Z"}]);
    view_tasks(
        &mut c.server,
        json!([{"id": 59, "title": "Yeni", "tasks": [late, early, task(3, "yok", 3.0)]}]),
    );
    let since = chrono::DateTime::parse_from_rfc3339("2026-01-05T09:00:00Z").unwrap();
    let found = c.plugin.reminders(RemindersParams { since }).unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].item.id, "1");
    assert_eq!(found[0].at.to_rfc3339(), "2026-01-05T09:30:00+00:00");
}

#[test]
fn server_error_has_short_message_without_body() {
    let mut c = ctx(None);
    mock(
        &mut c.server,
        "GET",
        "/api/v1/projects/9/views/37/tasks?per_page=200&page=1",
    )
    .with_status(500)
    .with_body(format!("stack trace {TOKEN}"))
    .create();
    let err = c
        .plugin
        .list(ListParams {
            view: View::Summary,
        })
        .unwrap_err();
    assert_eq!(err, "Vikunja 500: server error");
}

fn tasks_page(server: &mut ServerGuard, page: u32, columns: Value) -> Mock {
    mock(
        server,
        "GET",
        &format!("/api/v1/projects/9/views/37/tasks?per_page=200&page={page}"),
    )
    .with_body(columns.to_string())
    .create()
}

fn numbered(range: std::ops::RangeInclusive<u64>) -> Vec<Value> {
    range.map(|id| task(id, "t", id as f64)).collect()
}

#[test]
fn configure_finds_project_on_later_page() {
    let mut server = mockito::Server::new();
    let keyfile = keyfile_new();
    let mut page = |n: u32, body: Value| {
        mock(
            &mut server,
            "GET",
            &format!("/api/v1/projects?per_page=200&page={n}"),
        )
        .with_body(body.to_string())
        .create()
    };
    let _p1 = page(1, json!([{"id": 3, "title": "Başka"}]));
    let _p2 = page(2, json!([{"id": 9, "title": "Home"}]));
    let _p3 = page(3, json!([]));
    let _views = mock(&mut server, "GET", "/api/v1/projects/9/views")
        .with_body(kanban_views().to_string())
        .create();
    let mut plugin = Vikunja::default();
    plugin
        .configure(config(&server, &keyfile, "Home", None))
        .unwrap();
}

#[test]
fn list_returns_tasks_beyond_the_first_page() {
    let mut c = ctx(None);
    tasks_page(
        &mut c.server,
        1,
        json!([{"id": 59, "title": "Yeni", "count": 250, "tasks": numbered(1..=200)}]),
    );
    tasks_page(
        &mut c.server,
        2,
        json!([{"id": 59, "title": "Yeni", "count": 250, "tasks": numbered(201..=250)}]),
    );
    let items = list(&mut c);
    assert_eq!(items.len(), 250);
    assert!(items.iter().any(|i| i.id == "250"));
}

#[test]
fn list_pages_on_when_the_server_sends_no_count() {
    let mut c = ctx(None);
    tasks_page(
        &mut c.server,
        1,
        json!([{"id": 59, "title": "Yeni", "tasks": numbered(1..=200)}]),
    );
    tasks_page(
        &mut c.server,
        2,
        json!([{"id": 59, "title": "Yeni", "tasks": numbered(201..=230)}]),
    );
    assert_eq!(list(&mut c).len(), 230);
}

#[test]
fn list_pages_each_bucket_by_its_own_count() {
    let mut c = ctx(None);
    tasks_page(
        &mut c.server,
        1,
        json!([
            {"id": 59, "title": "Yeni", "count": 205, "tasks": numbered(1..=200)},
            {"id": 62, "title": "Karşı tarafı bekliyorum", "count": 1, "tasks": numbered(1000..=1000)},
        ]),
    );
    tasks_page(
        &mut c.server,
        2,
        json!([
            {"id": 59, "title": "Yeni", "count": 205, "tasks": numbered(201..=205)},
            {"id": 62, "title": "Karşı tarafı bekliyorum", "count": 1, "tasks": []},
        ]),
    );
    assert_eq!(list(&mut c).len(), 206);
}
