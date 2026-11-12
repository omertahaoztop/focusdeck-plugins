use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use reqwest::Url;

use crate::client::http;
use crate::i18n::{msg, t, t_args};
use crate::oauth::{
    pkce_challenge, random_token, read_client_file, request_token, token_error_message, write_token,
};

const SCOPE: &str = "https://www.googleapis.com/auth/gmail.readonly";
const CALLBACK_TIMEOUT: Duration = Duration::from_secs(300);
const MAX_REQUEST_BYTES: u64 = 16 * 1024;

pub struct LoginArgs {
    pub client_file: PathBuf,
    pub token_file: PathBuf,
    pub hint: Option<String>,
}

impl LoginArgs {
    pub fn parse(args: impl IntoIterator<Item = String>) -> Result<Self, String> {
        let usage_error = |reason: String| format!("{reason}\n{}", t(msg::USAGE));
        let mut client_file = None;
        let mut token_file = None;
        let mut hint = None;
        let mut args = args.into_iter();
        while let Some(flag) = args.next() {
            let slot = match flag.as_str() {
                "--client-file" => &mut client_file,
                "--token-file" => &mut token_file,
                "--hint" => &mut hint,
                other => {
                    return Err(usage_error(t_args(
                        msg::UNKNOWN_FLAG,
                        &[("flag", other.into())],
                    )));
                }
            };
            let value = args.next().ok_or_else(|| {
                usage_error(t_args(
                    msg::FLAG_VALUE_MISSING,
                    &[("flag", flag.as_str().into())],
                ))
            })?;
            *slot = Some(value);
        }
        let required = |value: Option<String>, flag: &str| {
            value.ok_or_else(|| usage_error(t_args(msg::FLAG_MISSING, &[("flag", flag.into())])))
        };
        Ok(Self {
            client_file: required(client_file, "--client-file")?.into(),
            token_file: required(token_file, "--token-file")?.into(),
            hint,
        })
    }
}

#[derive(Default)]
pub struct Callback {
    pub code: Option<String>,
    pub state: Option<String>,
    pub error: Option<String>,
}

pub fn parse_request_line(line: &str) -> Option<Callback> {
    let mut parts = line.split_whitespace();
    if parts.next()? != "GET" {
        return None;
    }
    let target = parts.next()?;
    if !target.starts_with('/') {
        return None;
    }
    let url = Url::parse(&format!("http://localhost{target}")).ok()?;
    let mut callback = Callback::default();
    for (key, value) in url.query_pairs() {
        let slot = match key.as_ref() {
            "code" => &mut callback.code,
            "state" => &mut callback.state,
            "error" => &mut callback.error,
            _ => continue,
        };
        *slot = Some(value.into_owned());
    }
    Some(callback)
}

pub fn check_callback(callback: Callback, expected_state: &str) -> Result<String, String> {
    match callback {
        Callback {
            code: Some(code),
            state: Some(state),
            error: None,
        } if state == expected_state => Ok(code),
        _ => Err(t(msg::DENIED)),
    }
}

pub fn wait_for_callback(
    listener: &TcpListener,
    expected_state: &str,
    timeout: Duration,
) -> Result<String, String> {
    listener
        .set_nonblocking(true)
        .map_err(|_| t(msg::LISTENER_SETUP_FAILED))?;
    let deadline = Instant::now() + timeout;
    loop {
        if Instant::now() >= deadline {
            return Err(t(msg::APPROVAL_TIMEOUT));
        }
        match listener.accept() {
            Ok((stream, _)) => {
                if let Some(result) = serve_connection(stream, expected_state) {
                    return result;
                }
            }
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(_) => return Err(t(msg::LISTENER_FAILED)),
        }
    }
}

fn serve_connection(stream: TcpStream, expected_state: &str) -> Option<Result<String, String>> {
    stream.set_nonblocking(false).ok()?;
    stream.set_read_timeout(Some(Duration::from_secs(2))).ok()?;
    stream
        .set_write_timeout(Some(Duration::from_secs(2)))
        .ok()?;
    let line = read_request_head(&stream)?;
    let callback = match parse_request_line(&line) {
        Some(cb) if cb.code.is_some() || cb.error.is_some() => cb,
        _ => {
            respond(stream, "404 Not Found", &t(msg::NOT_FOUND_PAGE));
            return None;
        }
    };
    let result = check_callback(callback, expected_state);
    match &result {
        Ok(_) => respond(stream, "200 OK", &t(msg::CONNECTED_PAGE)),
        Err(_) => respond(stream, "200 OK", &t(msg::DENIED_PAGE)),
    }
    Some(result)
}

fn read_request_head(stream: &TcpStream) -> Option<String> {
    let mut reader = BufReader::new(stream.take(MAX_REQUEST_BYTES));
    let mut first = String::new();
    reader.read_line(&mut first).ok()?;
    let mut header = String::new();
    // unread request bytes would turn the close into a reset and the browser would drop our reply
    while reader.read_line(&mut header).ok()? > 0 && !header.trim().is_empty() {
        header.clear();
    }
    Some(first)
}

fn respond(mut stream: TcpStream, status: &str, body: &str) {
    let reply = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(reply.as_bytes());
}

pub fn run(args: LoginArgs) -> Result<(), String> {
    let token_file = args.token_file.clone();
    run_with(args, |url| {
        eprintln!("{}", t(msg::OPEN_PROMPT));
        println!("{url}");
        let _ = Command::new("xdg-open")
            .arg(url)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
    })?;
    println!(
        "{}",
        t_args(
            msg::CONNECTED,
            &[("path", token_file.display().to_string().as_str().into())]
        )
    );
    Ok(())
}

pub fn run_with(args: LoginArgs, announce: impl FnOnce(&str)) -> Result<(), String> {
    let secret = read_client_file(&args.client_file)?;
    let verifier = random_token(32)?;
    let state = random_token(16)?;
    let listener =
        TcpListener::bind(("127.0.0.1", 0)).map_err(|_| t(msg::LISTENER_SETUP_FAILED))?;
    let port = listener
        .local_addr()
        .map_err(|_| t(msg::LISTENER_SETUP_FAILED))?
        .port();
    let redirect_uri = format!("http://127.0.0.1:{port}/");

    let mut auth_url = Url::parse(&secret.auth_uri)
        .ok()
        .filter(|u| matches!(u.scheme(), "https" | "http"))
        .ok_or_else(|| t(msg::AUTH_URI_INVALID))?;
    {
        let challenge = pkce_challenge(&verifier);
        let mut query = auth_url.query_pairs_mut();
        query.extend_pairs([
            ("client_id", secret.client_id.as_str()),
            ("redirect_uri", redirect_uri.as_str()),
            ("response_type", "code"),
            ("scope", SCOPE),
            ("access_type", "offline"),
            ("prompt", "consent"),
            ("state", state.as_str()),
            ("code_challenge", challenge.as_str()),
            ("code_challenge_method", "S256"),
        ]);
        if let Some(hint) = &args.hint {
            query.append_pair("login_hint", hint);
        }
    }
    announce(auth_url.as_str());

    let code = wait_for_callback(&listener, &state, CALLBACK_TIMEOUT)?;
    drop(listener);

    let grant = [
        ("grant_type", "authorization_code"),
        ("code", code.as_str()),
        ("code_verifier", verifier.as_str()),
        ("redirect_uri", redirect_uri.as_str()),
    ];
    let response = request_token(&http()?, &secret, &grant).map_err(token_error_message)?;
    let refresh_token = response
        .refresh_token
        .filter(|t| !t.trim().is_empty())
        .ok_or_else(|| t(msg::NO_REFRESH_TOKEN))?;
    write_token(&args.token_file, &refresh_token).map_err(|_| {
        t_args(
            msg::TOKEN_WRITE_FAILED,
            &[(
                "path",
                args.token_file.display().to_string().as_str().into(),
            )],
        )
    })
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::time::Duration;

    use super::*;

    fn args(list: &[&str]) -> Result<LoginArgs, String> {
        LoginArgs::parse(list.iter().map(|s| s.to_string()))
    }

    #[test]
    fn callback_parses_code_and_state() {
        let cb = parse_request_line("GET /?code=4%2Fabc&state=xyz HTTP/1.1").unwrap();
        assert_eq!(cb.code.as_deref(), Some("4/abc"));
        assert_eq!(cb.state.as_deref(), Some("xyz"));
        assert_eq!(cb.error, None);

        let denied = parse_request_line("GET /?error=access_denied&state=xyz HTTP/1.1").unwrap();
        assert_eq!(denied.error.as_deref(), Some("access_denied"));
        assert_eq!(denied.code, None);

        let favicon = parse_request_line("GET /favicon.ico HTTP/1.1").unwrap();
        assert!(favicon.code.is_none() && favicon.error.is_none());

        assert!(parse_request_line("POST /?code=a HTTP/1.1").is_none());
        assert!(parse_request_line("garbage").is_none());
    }

    #[test]
    fn mismatched_state_is_rejected() {
        let cb = parse_request_line("GET /?code=abc&state=other HTTP/1.1").unwrap();
        assert_eq!(
            check_callback(cb, "xyz").unwrap_err(),
            "authorization denied"
        );
        let no_state = parse_request_line("GET /?code=abc HTTP/1.1").unwrap();
        assert_eq!(
            check_callback(no_state, "xyz").unwrap_err(),
            "authorization denied"
        );
        let denied = parse_request_line("GET /?error=access_denied&state=xyz HTTP/1.1").unwrap();
        assert_eq!(
            check_callback(denied, "xyz").unwrap_err(),
            "authorization denied"
        );
        let good = parse_request_line("GET /?code=abc&state=xyz HTTP/1.1").unwrap();
        assert_eq!(check_callback(good, "xyz").unwrap(), "abc");
    }

    fn request(port: u16, line: &str) -> String {
        let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
        write!(stream, "{line}\r\nHost: localhost\r\n\r\n").unwrap();
        let mut out = String::new();
        stream.read_to_string(&mut out).unwrap();
        out
    }

    #[test]
    fn requests_without_code_are_ignored_until_the_callback_arrives() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let client = std::thread::spawn(move || {
            let ico = request(port, "GET /favicon.ico HTTP/1.1");
            let ok = request(port, "GET /?code=abc&state=xyz HTTP/1.1");
            (ico, ok)
        });
        let code = wait_for_callback(&listener, "xyz", Duration::from_secs(10)).unwrap();
        assert_eq!(code, "abc");
        let (ico, ok) = client.join().unwrap();
        assert!(ico.starts_with("HTTP/1.1 404"), "{ico}");
        assert!(ok.starts_with("HTTP/1.1 200"), "{ok}");
        assert!(
            ok.ends_with("focusdeck: Gmail connected, you can close this tab."),
            "{ok}"
        );
    }

    #[test]
    fn missing_callback_times_out() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let err = wait_for_callback(&listener, "xyz", Duration::from_millis(200)).unwrap_err();
        assert_eq!(err, "No approval within 5 minutes");
    }

    #[test]
    fn login_flags_parse() {
        let parsed = args(&[
            "--client-file",
            "/c",
            "--token-file",
            "/t",
            "--hint",
            "a@example.com",
        ])
        .unwrap();
        assert_eq!(parsed.client_file, std::path::PathBuf::from("/c"));
        assert_eq!(parsed.token_file, std::path::PathBuf::from("/t"));
        assert_eq!(parsed.hint.as_deref(), Some("a@example.com"));
        let no_hint = args(&["--token-file", "/t", "--client-file", "/c"]).unwrap();
        assert_eq!(no_hint.hint, None);
    }

    #[test]
    fn bad_login_flags_print_usage() {
        for list in [
            &["--client-file", "/c"][..],
            &["--client-file", "/c", "--token-file"],
            &["--client-file", "/c", "--token-file", "/t", "--nope", "x"],
        ] {
            let err = args(list).err().unwrap();
            assert!(
                err.contains("focusdeck-gmail login --client-file <path> --token-file <path>"),
                "{err}"
            );
        }
    }
}
