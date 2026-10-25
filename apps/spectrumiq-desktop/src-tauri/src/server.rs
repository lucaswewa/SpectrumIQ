//! Where the local service is, and whether it answers.

use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use url::Url;

/// The local service's address, unless `--server` or `SPECTRUMIQ_SERVER` gives another.
pub const DEFAULT_SERVER: &str = "http://127.0.0.1:5000/";
/// The route the shell probes: `spectrumiq-server`'s health.
pub const HEALTH_PATH: &str = "/api/siq/system/health";

/// Why an address isn't one the shell can load.
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum UrlError {
    /// It doesn't parse as a URL.
    #[error("{0:?} isn't a URL")]
    Invalid(String),
    /// It isn't plain HTTP on this computer, the only origin the shell's remote capability allows.
    #[error("{0} isn't on this computer: the shell loads only http://127.0.0.1:<port> (ADR-0040)")]
    NotLocal(String),
}

/// The service's base URL from `text`: `http://127.0.0.1:<port>/`, with `localhost` taken as
/// 127.0.0.1, and any path, query or fragment dropped.
pub fn server_url(text: &str) -> Result<Url, UrlError> {
    let mut url = Url::parse(text).map_err(|_| UrlError::Invalid(text.to_owned()))?;
    let local = matches!(url.host_str(), Some("127.0.0.1" | "localhost"));
    if url.scheme() != "http" || !local || url.port().is_none() {
        return Err(UrlError::NotLocal(text.to_owned()));
    }
    url.set_host(Some("127.0.0.1"))
        .map_err(|_| UrlError::Invalid(text.to_owned()))?;
    url.set_path("/");
    url.set_query(None);
    url.set_fragment(None);
    Ok(url)
}

/// The service's address: `--server <url>` (or `--server=<url>`) among `args`, else `env`
/// (`SPECTRUMIQ_SERVER`), else [`DEFAULT_SERVER`].
pub fn configured(
    args: impl IntoIterator<Item = String>,
    env: Option<String>,
) -> Result<Url, UrlError> {
    let mut args = args.into_iter();
    let mut given = None;
    while let Some(arg) = args.next() {
        if arg == "--server" {
            given = args.next();
        } else if let Some(value) = arg.strip_prefix("--server=") {
            given = Some(value.to_owned());
        }
    }
    server_url(&given.or(env).unwrap_or_else(|| DEFAULT_SERVER.to_owned()))
}

/// What the health route answers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Health {
    /// `ok` while the service runs.
    pub status: String,
    /// The service's version.
    pub version: String,
    /// The camera it drives, if any.
    #[serde(default)]
    pub camera: Option<String>,
}

/// Why the service didn't answer as `spectrumiq-server` does.
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum ProbeError {
    /// Nothing accepted the connection, or nothing answered in time.
    #[error("nothing answers at {0}")]
    Unreachable(String),
    /// It answered with an error status.
    #[error("{0} answered {1:?}")]
    Status(String, String),
    /// It answered, but not with SpectrumIQ's health.
    #[error("{0} answered, but not as SpectrumIQ does: {1}")]
    NotSpectrumIq(String, String),
}

/// Asks the service at `base` for its health, giving up after `timeout`. One HTTP/1.1 request
/// over loopback, which needs no HTTP client.
pub async fn probe(base: &Url, timeout: Duration) -> Result<Health, ProbeError> {
    let at = base.to_string();
    let host = base.host_str().unwrap_or("127.0.0.1");
    let address = format!("{host}:{}", base.port_or_known_default().unwrap_or(80));
    let exchange = async {
        let mut stream = TcpStream::connect(&address).await?;
        let request = format!(
            "GET {HEALTH_PATH} HTTP/1.1\r\nHost: {address}\r\nAccept: application/json\r\n\
             Connection: close\r\n\r\n"
        );
        stream.write_all(request.as_bytes()).await?;
        let mut answer = Vec::new();
        stream.read_to_end(&mut answer).await?;
        Ok::<_, std::io::Error>(answer)
    };
    match tokio::time::timeout(timeout, exchange).await {
        Ok(Ok(answer)) => health(&at, &answer),
        Ok(Err(_)) | Err(_) => Err(ProbeError::Unreachable(at)),
    }
}

/// The health in an HTTP answer: a 200, with SpectrumIQ's JSON as its body.
fn health(at: &str, answer: &[u8]) -> Result<Health, ProbeError> {
    let text = String::from_utf8_lossy(answer);
    let not_ours = |why: &str| ProbeError::NotSpectrumIq(at.to_owned(), why.to_owned());
    let (head, body) = text
        .split_once("\r\n\r\n")
        .ok_or_else(|| not_ours("no HTTP answer"))?;
    let status = head.lines().next().unwrap_or_default();
    if status.split_whitespace().nth(1) != Some("200") {
        return Err(ProbeError::Status(at.to_owned(), status.to_owned()));
    }
    serde_json::from_str(body.trim()).map_err(|error| not_ours(&error.to_string()))
}

#[cfg(test)]
mod tests;
