use std::time::Duration;

use seenboard_plugin_common::http_client;

#[test]
fn plain_http_to_this_machine_skips_the_environment_proxy() {
    let mut proxy = mockito::Server::new();
    let leaked = proxy.mock("GET", mockito::Matcher::Any).expect(0).create();
    let mut target = mockito::Server::new();
    let reached = target.mock("GET", "/ping").create();
    // the only test in this binary, so no other test reads the environment while it changes
    unsafe {
        std::env::set_var("http_proxy", proxy.url());
        std::env::remove_var("no_proxy");
        std::env::remove_var("NO_PROXY");
    }
    let url = format!("{}/ping", target.url());
    let http = http_client("test/0", Duration::from_secs(5), &[&url])
        .build()
        .unwrap();
    http.get(&url).send().unwrap();
    reached.assert();
    leaked.assert();
}
