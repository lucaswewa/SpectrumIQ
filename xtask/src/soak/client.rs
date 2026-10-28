//! The soak's clients: HTTP/1.0 requests with a session's cookie, whose answers end when the
//! connection closes, so that no chunked body needs decoding; an MJPEG stream's frames; and a
//! Thing's observation socket.

use std::io::{ErrorKind, Read, Write};
use std::net::TcpStream;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail, ensure};
use serde_json::{Map, Value, json};
use tungstenite::client::IntoClientRequest;
use tungstenite::http::HeaderValue;
use tungstenite::{Error as SocketError, Message};

/// Where the soak's service listens: its own port, not the default service's.
pub(crate) const PORT: u16 = 5097;
/// How long a read waits before the soak looks at the clock again.
const TICK: Duration = Duration::from_secs(1);
/// What begins each part of teta-wot's MJPEG streams.
const FRAME: &[u8] = b"--frame\r\n";

fn address() -> String {
    format!("127.0.0.1:{PORT}")
}

/// A client: its requests carry the session's cookie, once logged in.
#[derive(Debug, Default)]
pub(crate) struct Session {
    cookie: Option<String>,
}

/// What a viewer saw of a stream: its frames, and the gaps between them in ms.
#[derive(Debug, Default)]
pub(crate) struct Frames {
    pub(crate) count: u64,
    pub(crate) gaps_ms: Vec<f64>,
}

impl Session {
    /// Logs `name` in with `password`.
    pub(crate) fn login(name: &str, password: &str) -> Result<Session> {
        let body = json!({ "name": name, "password": password });
        let (status, head, answer) =
            Session::default().send("POST", "/api/siq/auth/login", Some(&body))?;
        ensure!(
            status == 200,
            "logging in as {name}: {status} {}",
            String::from_utf8_lossy(&answer)
        );
        let cookie = head.lines().find_map(|line| {
            let (header, value) = line.split_once(':')?;
            let cookie = value.trim().split(';').next()?;
            header
                .eq_ignore_ascii_case("set-cookie")
                .then(|| cookie.to_owned())
        });
        Ok(Session {
            cookie: Some(cookie.context("the login set no cookie")?),
        })
    }

    /// Sends a request, and reads its answer to the end: its status, headers and body.
    fn send(
        &self,
        method: &str,
        path: &str,
        body: Option<&Value>,
    ) -> Result<(u16, String, Vec<u8>)> {
        let mut stream = self.open(method, path, body)?;
        let mut answer = Vec::new();
        stream.read_to_end(&mut answer)?;
        let end = find(&answer, b"\r\n\r\n").context("no HTTP answer")?;
        let head = String::from_utf8_lossy(&answer[..end]).into_owned();
        let status = head.split(' ').nth(1).and_then(|code| code.parse().ok());
        Ok((
            status.context("no status")?,
            head,
            answer[end + 4..].to_vec(),
        ))
    }

    /// Connects, and sends a request: the connection, to read the answer from.
    fn open(&self, method: &str, path: &str, body: Option<&Value>) -> Result<TcpStream> {
        let mut stream = TcpStream::connect(address())?;
        let body = body.map(Value::to_string).unwrap_or_default();
        let mut request = format!(
            "{method} {path} HTTP/1.0\r\nHost: {}\r\nContent-Type: application/json\r\n\
             Content-Length: {}\r\n",
            address(),
            body.len()
        );
        if let Some(cookie) = &self.cookie {
            request += &format!("Cookie: {cookie}\r\n");
        }
        request += "\r\n";
        request += &body;
        stream.write_all(request.as_bytes())?;
        Ok(stream)
    }

    /// The JSON answer to a request, `null` for none, or what was wrong.
    pub(crate) fn json(&self, method: &str, path: &str, body: Option<&Value>) -> Result<Value> {
        let (status, _, answer) = self.send(method, path, body)?;
        ensure!(
            (200..300).contains(&status),
            "{method} {path}: {status} {}",
            String::from_utf8_lossy(&answer)
        );
        if answer.is_empty() {
            return Ok(Value::Null);
        }
        serde_json::from_slice(&answer).with_context(|| format!("{method} {path}"))
    }

    /// The text of `path`.
    pub(crate) fn text(&self, path: &str) -> Result<String> {
        let (status, _, answer) = self.send("GET", path, None)?;
        ensure!(status == 200, "GET {path}: {status}");
        Ok(String::from_utf8_lossy(&answer).into_owned())
    }

    /// How long `path` took to fetch, in ms, or `None` if it failed.
    pub(crate) fn timed(&self, path: &str) -> Option<f64> {
        let started = Instant::now();
        let fetched = self.send("GET", path, None);
        matches!(fetched, Ok((200, _, _))).then(|| started.elapsed().as_secs_f64() * 1e3)
    }

    /// Invokes `action`, as `thing/action`, with `input`, and waits for its output. Still running
    /// at `until`, it is cancelled, and gives `None`.
    pub(crate) fn invoke(
        &self,
        action: &str,
        input: &Value,
        until: Instant,
    ) -> Result<Option<Value>> {
        let mut invocation = self.json("POST", &format!("/api/v1/{action}"), Some(input))?;
        loop {
            match invocation["status"].as_str() {
                Some("completed") => return Ok(Some(invocation["output"].take())),
                Some("pending" | "running") => {}
                _ => bail!("{action}: {invocation}"),
            }
            let href = invocation["href"]
                .as_str()
                .context("an invocation without href")?;
            let path = &href[href.find("/api/").context("an invocation's href")?..];
            if Instant::now() >= until {
                self.json("DELETE", path, None)?;
                return Ok(None);
            }
            thread::sleep(Duration::from_millis(250));
            invocation = self.json("GET", path, None)?;
        }
    }

    /// Follows the MJPEG stream at `path` until `until`.
    pub(crate) fn frames(&self, path: &str, until: Instant) -> Result<Frames> {
        let mut stream = self.open("GET", path, None)?;
        stream.set_read_timeout(Some(TICK))?;
        let (mut seen, mut last) = (Frames::default(), None);
        let (mut pending, mut buffer) = (Vec::new(), vec![0; 1 << 16]);
        while Instant::now() < until {
            let read = match stream.read(&mut buffer) {
                Ok(0) => bail!("{path} ended"),
                Ok(read) => read,
                Err(error) if waited(error.kind()) => continue,
                Err(error) => return Err(error.into()),
            };
            for _ in 0..parts(&mut pending, &buffer[..read]) {
                let now = Instant::now();
                if let Some(before) = last.replace(now) {
                    seen.gaps_ms.push((now - before).as_secs_f64() * 1e3);
                }
                seen.count += 1;
            }
        }
        Ok(seen)
    }

    /// Observes `properties` of `thing` over its socket until `until`: the messages received.
    pub(crate) fn observe(&self, thing: &str, properties: &[&str], until: Instant) -> Result<u64> {
        let mut request = format!("ws://{}/api/v1/{thing}/ws", address()).into_client_request()?;
        if let Some(cookie) = &self.cookie {
            request
                .headers_mut()
                .insert("Cookie", HeaderValue::from_str(cookie)?);
        }
        let stream = TcpStream::connect(address())?;
        let (mut socket, _) =
            tungstenite::client(request, stream).map_err(|error| anyhow!("{thing}/ws: {error}"))?;
        socket.get_ref().set_read_timeout(Some(TICK))?;
        let names: Map<String, Value> = (properties.iter())
            .map(|name| ((*name).to_owned(), Value::Bool(true)))
            .collect();
        let subscribe = json!({ "messageType": "addPropertyObservation", "data": names });
        socket.send(Message::text(subscribe.to_string()))?;
        let mut messages = 0;
        while Instant::now() < until {
            match socket.read() {
                Ok(Message::Text(_)) => messages += 1,
                Ok(_) => {}
                Err(SocketError::Io(error)) if waited(error.kind()) => {}
                Err(error) => bail!("{thing}/ws: {error}"),
            }
        }
        socket.close(None).ok();
        Ok(messages)
    }
}

/// How many MJPEG parts begin in `chunk`, after what `pending` kept of the chunk before: it keeps
/// what could begin the next part's boundary.
pub(crate) fn parts(pending: &mut Vec<u8>, chunk: &[u8]) -> usize {
    pending.extend_from_slice(chunk);
    let mut parts = 0;
    while let Some(at) = find(pending, FRAME) {
        parts += 1;
        pending.drain(..at + FRAME.len());
    }
    let keep = pending.len().min(FRAME.len() - 1);
    pending.drain(..pending.len() - keep);
    parts
}

/// Whether a read ended for its timeout, which is `WouldBlock` or `TimedOut` by platform.
fn waited(kind: ErrorKind) -> bool {
    matches!(kind, ErrorKind::WouldBlock | ErrorKind::TimedOut)
}

/// Where `needle` first is in `haystack`.
fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}
