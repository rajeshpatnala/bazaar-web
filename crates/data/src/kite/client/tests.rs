//! Mock servers on 127.0.0.1 only; no call leaves the machine.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread;

use bazaar_core::config::{DataConfig, parse_str};
use wiremock::matchers::{header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::*;

const KEY: &str = "KEY-zzz111";
const TOKEN: &str = "TOKEN-zzz222";

fn shipped() -> KiteSection {
    let text = include_str!("../../../../../config/data.toml");
    parse_str::<DataConfig>(text).unwrap().kite.unwrap()
}

fn local(base: &str) -> KiteSection {
    KiteSection {
        api_base: base.to_owned(),
        allowed_hosts: vec!["127.0.0.1".to_owned()],
        connect_timeout_secs: 1,
        request_timeout_secs: 1,
        max_response_bytes: 256,
        ..shipped()
    }
}

fn client(base: &str) -> KiteClient {
    KiteClient::build(&local(base), Scheme::LoopbackHttp).unwrap()
}

/// One signed GET of `/x` as JSON.
async fn call(client: &KiteClient) -> Result<Vec<u32>, KiteError> {
    let (api_key, access_token) = (Secret::new(KEY.into()), Secret::new(TOKEN.into()));
    let credentials = KiteCredentials {
        api_key: &api_key,
        access_token: &access_token,
    };
    client.get_json("/x", &[], credentials).await
}

async fn serve(response: ResponseTemplate) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(response)
        .mount(&server)
        .await;
    server
}

async fn answer(status: u16, body: &str) -> Result<Vec<u32>, KiteError> {
    let server = serve(ResponseTemplate::new(status).set_body_string(body)).await;
    call(&client(&server.uri())).await
}

/// A server that sends `reply` exactly, then holds the socket open for `hold_ms`.
fn raw_server(reply: String, hold_ms: u64) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        let _read = socket.read(&mut [0; 4096]).unwrap();
        socket.write_all(reply.as_bytes()).unwrap();
        thread::sleep(Duration::from_millis(hold_ms));
    });
    base
}

#[test]
fn production_client_takes_the_shipped_config_and_refuses_other_targets() {
    assert!(KiteClient::new(&shipped()).is_ok());
    for base in [
        "http://api.kite.trade",
        "https://evil.example",
        "https://api.kite.trade.evil.example",
        "https://user@api.kite.trade",
        "https://:pw@api.kite.trade",
        "https://api.kite.trade:8443",
        "http://127.0.0.1:9",
        "not a url",
    ] {
        let mut config = shipped();
        config.api_base = base.to_owned();
        let got = KiteClient::new(&config).unwrap_err();
        assert_eq!(got, KiteError::DisallowedTarget, "{base}");
    }
    for base in ["https://api.kite.trade/v9", "https://api.kite.trade/?a=1"] {
        let mut config = shipped();
        config.api_base = base.to_owned();
        let got = KiteClient::new(&config).unwrap_err();
        assert_eq!(got, KiteError::Config("api_base must end at the host"));
    }
}

#[test]
fn unbounded_settings_are_refused() {
    let broken: [fn(&mut KiteSection); 3] = [
        |c| c.connect_timeout_secs = 0,
        |c| c.request_timeout_secs = 0,
        |c| c.max_response_bytes = 0,
    ];
    for change in broken {
        let mut config = shipped();
        change(&mut config);
        let got = KiteClient::new(&config);
        assert!(matches!(got, Err(KiteError::Config(_))), "{got:?}");
    }
}

#[test]
fn a_path_cannot_change_the_target() {
    let client = KiteClient::new(&shipped()).unwrap();
    for bad in [
        "quote",
        "//evil.example/x",
        "https://evil.example/x",
        "/a/../b",
        "/a/./b",
        "/a/%2e%2e/b",
        "/a?x=1",
        "/a#f",
        "/\\evil.example",
        "/a b",
    ] {
        let got = client.url(bad, &[]).unwrap_err();
        assert_eq!(got, KiteError::InvalidRequest("path is not a plain path"));
    }
    let url = client.url("/quote", &[("i", "NSE:M&M"), ("i", "NSE:X")]);
    assert_eq!(
        url.unwrap().as_str(),
        "https://api.kite.trade/quote?i=NSE%3AM%26M&i=NSE%3AX"
    );
}

#[tokio::test]
async fn signed_get_sends_headers_and_query_and_unwraps_data() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/x"))
        .and(query_param("i", "NSE:X"))
        .and(header("X-Kite-Version", "3"))
        .and(header(
            "Authorization",
            format!("token {KEY}:{TOKEN}").as_str(),
        ))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(r#"{"status":"success","data":[7]}"#),
        )
        .expect(1)
        .mount(&server)
        .await;
    let (api_key, access_token) = (Secret::new(KEY.into()), Secret::new(TOKEN.into()));
    let credentials = KiteCredentials {
        api_key: &api_key,
        access_token: &access_token,
    };
    let got: Vec<u32> = client(&server.uri())
        .get_json("/x", &[("i", "NSE:X")], credentials)
        .await
        .unwrap();
    assert_eq!(got, [7]);
}

#[tokio::test]
async fn a_blank_or_unsendable_credential_sends_nothing() {
    let server = MockServer::start().await;
    let client = client(&server.uri());
    let unsendable = KiteError::InvalidRequest("credential is not a header value");
    for (key, token, want) in [
        ("", TOKEN, KiteError::LoginRequired),
        (KEY, "  ", KiteError::LoginRequired),
        (KEY, "a\r\nb", unsendable),
    ] {
        let (api_key, access_token) = (Secret::new(key.into()), Secret::new(token.into()));
        let credentials = KiteCredentials {
            api_key: &api_key,
            access_token: &access_token,
        };
        let got = client.get_bytes("/x", &[], credentials).await;
        assert_eq!(got.unwrap_err(), want);
    }
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn every_failing_status_is_a_typed_error() {
    use ApiErrorType::{General, Input, Missing, Other, Permission, Token};
    let body = |kind: &str| format!(r#"{{"status":"error","error_type":"{kind}"}}"#);
    let rejected = |status, kind| KiteError::Rejected { status, kind };
    let upstream = |status, kind| KiteError::Upstream { status, kind };
    for (status, text, want) in [
        (403, body("TokenException"), KiteError::LoginRequired),
        (403, body("PermissionException"), rejected(403, Permission)),
        (403, "<html>".into(), rejected(403, Missing)),
        // Too large to read: the status still decides.
        (403, "x".repeat(300), rejected(403, Missing)),
        (400, body("InputException"), rejected(400, Input)),
        (404, body("NewException"), rejected(404, Other)),
        (429, String::new(), KiteError::RateLimited),
        (500, body("GeneralException"), upstream(500, General)),
        (503, body("TokenException"), upstream(503, Token)),
        // A success status other than 200 may be a partial body.
        (
            206,
            r#"{"status":"success","data":[]}"#.into(),
            rejected(206, Missing),
        ),
        (200, body("InputException"), rejected(200, Input)),
        (200, r#"{"data":[1]}"#.into(), rejected(200, Missing)),
    ] {
        assert_eq!(answer(status, &text).await, Err(want), "{status} {text}");
    }
}

#[tokio::test]
async fn bad_json_is_a_decode_error() {
    for text in [
        "",
        "{",
        "<html>",
        r#"{"status":"success"}"#,
        r#"{"status":"success","data":"x"}"#,
    ] {
        let got = answer(200, text).await;
        assert!(matches!(got, Err(KiteError::Decode { .. })), "{got:?}");
    }
}

#[tokio::test]
async fn a_redirect_is_never_followed() {
    // The destination is on the allowlist too, so only the redirect policy stops it.
    let destination = serve(ResponseTemplate::new(200)).await;
    for status in [301, 302, 307, 308] {
        let location = format!("{}/x", destination.uri());
        let reply = ResponseTemplate::new(status).insert_header("Location", location.as_str());
        let server = serve(reply).await;
        let got = call(&client(&server.uri())).await;
        assert_eq!(got, Err(KiteError::Redirect { status }));
    }
    assert!(destination.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn a_failed_call_is_not_repeated() {
    let server = serve(ResponseTemplate::new(503)).await;
    let _failed = call(&client(&server.uri())).await.unwrap_err();
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
}

#[tokio::test]
async fn a_body_over_the_cap_is_refused_with_or_without_a_length() {
    let too_large = Err(KiteError::TooLarge { limit: 256 });
    assert_eq!(answer(200, &"x".repeat(257)).await, too_large);
    let chunk = format!("80\r\n{}\r\n", "x".repeat(128));
    let head = "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n";
    // Never terminated: the cap, not the end of the stream, must stop the read.
    let base = raw_server(format!("{head}{chunk}{chunk}{chunk}"), 3000);
    assert_eq!(call(&client(&base)).await, too_large);
    // Exactly at the cap is accepted.
    let exact = format!("{:256}", r#"{"status":"success","data":[1]}"#);
    assert_eq!(answer(200, &exact).await, Ok(vec![1]));
}

#[tokio::test]
async fn slow_headers_and_a_stalled_body_time_out() {
    let slow = ResponseTemplate::new(200).set_delay(Duration::from_secs(5));
    let server = serve(slow).await;
    let got = call(&client(&server.uri())).await;
    assert_eq!(got, Err(KiteError::Timeout));
    let stalled = "HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\n{";
    let base = raw_server(stalled.into(), 3000);
    assert_eq!(call(&client(&base)).await, Err(KiteError::Timeout));
}

#[tokio::test]
async fn a_cut_body_or_refused_connection_is_a_transport_error() {
    let cut = "HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\n{";
    let base = raw_server(cut.into(), 0);
    assert_eq!(call(&client(&base)).await, Err(KiteError::Transport));
    let closed = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", closed.local_addr().unwrap());
    drop(closed);
    assert_eq!(call(&client(&base)).await, Err(KiteError::Transport));
}

#[tokio::test]
async fn no_error_or_debug_text_shows_a_credential_or_the_body() {
    let echo = format!(r#"{{"status":"success","data":"{TOKEN}"}}"#);
    let hostile = format!(r#"{{"status":"error","error_type":"{TOKEN}","message":"{KEY}"}}"#);
    let mut errors = Vec::new();
    for (status, body) in [
        (200, &echo),
        (200, &hostile),
        (403, &hostile),
        (500, &hostile),
    ] {
        errors.push(answer(status, body).await.unwrap_err());
    }
    for error in errors {
        let text = format!("{error} {error:?}");
        assert!(!text.contains("zzz"), "{text}");
    }
    let (api_key, access_token) = (Secret::new(KEY.into()), Secret::new(TOKEN.into()));
    let credentials = KiteCredentials {
        api_key: &api_key,
        access_token: &access_token,
    };
    let sent = authorization(credentials).unwrap();
    let client = KiteClient::new(&shipped()).unwrap();
    let text = format!("{credentials:?} {sent:?} {client:?}");
    assert!(!text.contains("zzz"), "{text}");
}
