// REQ-NET-001: the shell finds the local service, and checks it answers as SpectrumIQ (ADR-0040).
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use super::*;

#[test]
fn an_address_is_the_service_s_root_on_this_computer() {
    let url = |text: &str| server_url(text).map(|url| url.to_string());
    assert_eq!(
        url("http://127.0.0.1:5000"),
        Ok("http://127.0.0.1:5000/".into())
    );
    // localhost is 127.0.0.1, the origin the remote capability allows; the path is dropped.
    assert_eq!(
        url("http://localhost:5078/#/live?x=1"),
        Ok("http://127.0.0.1:5078/".into())
    );
    for elsewhere in [
        "https://127.0.0.1:5000",
        "http://10.0.0.5:5000",
        "http://lab-pc:5000",
        "http://127.0.0.1",
    ] {
        assert_eq!(
            url(elsewhere),
            Err(UrlError::NotLocal(elsewhere.into())),
            "{elsewhere}"
        );
    }
    assert_eq!(
        url("127.0.0.1 port 5000"),
        Err(UrlError::Invalid("127.0.0.1 port 5000".into()))
    );
}

#[test]
fn the_address_comes_from_the_command_line_then_the_environment_then_the_default() {
    let args = |list: &[&str]| list.iter().map(|arg| arg.to_string()).collect::<Vec<_>>();
    let env = Some("http://127.0.0.1:6000".to_owned());
    let at = |list: &[&str], env: Option<String>| configured(args(list), env).unwrap().to_string();
    assert_eq!(
        at(&["--server", "http://127.0.0.1:7000"], env.clone()),
        "http://127.0.0.1:7000/"
    );
    assert_eq!(
        at(&["--server=http://localhost:7001"], env.clone()),
        "http://127.0.0.1:7001/"
    );
    assert_eq!(at(&["--other"], env), "http://127.0.0.1:6000/");
    assert_eq!(at(&[], None), DEFAULT_SERVER);
    assert!(configured(args(&["--server", "http://lab-pc:5000"]), None).is_err());
}

/// A server on a free port that answers one request with `answer`, and returns what it was asked.
async fn answering(answer: &'static str) -> (Url, tokio::task::JoinHandle<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = server_url(&format!("http://{}", listener.local_addr().unwrap())).unwrap();
    let asked = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut request = vec![0; 1024];
        let read = stream.read(&mut request).await.unwrap();
        stream.write_all(answer.as_bytes()).await.unwrap();
        String::from_utf8_lossy(&request[..read]).into_owned()
    });
    (url, asked)
}

const SECOND: Duration = Duration::from_secs(1);

#[tokio::test]
async fn a_healthy_service_answers_its_health() {
    let body = r#"{"status":"ok","version":"0.1.0","uptime_s":12.5,"camera":"simulated camera"}"#;
    let answer: &'static str = Box::leak(
        format!(
            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{body}",
            body.len()
        )
        .into_boxed_str(),
    );
    let (url, asked) = answering(answer).await;
    let health = probe(&url, SECOND).await.unwrap();
    assert_eq!(
        health,
        Health {
            status: "ok".into(),
            version: "0.1.0".into(),
            camera: Some("simulated camera".into())
        }
    );
    let request = asked.await.unwrap();
    assert!(
        request.starts_with("GET /api/siq/system/health HTTP/1.1\r\n"),
        "{request}"
    );
}

#[tokio::test]
async fn an_error_another_program_or_silence_is_not_a_healthy_service() {
    let (url, _) = answering("HTTP/1.1 503 Service Unavailable\r\ncontent-length: 0\r\n\r\n").await;
    let at = url.to_string();
    let status = "HTTP/1.1 503 Service Unavailable".to_owned();
    assert_eq!(
        probe(&url, SECOND).await,
        Err(ProbeError::Status(at, status))
    );

    let (url, _) = answering("HTTP/1.1 200 OK\r\n\r\n<html>another program</html>").await;
    assert!(matches!(
        probe(&url, SECOND).await,
        Err(ProbeError::NotSpectrumIq(..))
    ));

    // A port that accepts but never answers, then one that nothing listens on.
    let silent = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = server_url(&format!("http://{}", silent.local_addr().unwrap())).unwrap();
    let quick = Duration::from_millis(200);
    assert_eq!(
        probe(&url, quick).await,
        Err(ProbeError::Unreachable(url.to_string()))
    );
    drop(silent);
    assert_eq!(
        probe(&url, quick).await,
        Err(ProbeError::Unreachable(url.to_string()))
    );
}
