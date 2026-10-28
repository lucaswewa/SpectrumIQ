//! Where the local service is, whether it answers, and the local console's secret (ADR-0066).

use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use url::Url;

/// The local service's address, unless `--server` or `SPECTRUMIQ_SERVER` gives another.
pub const DEFAULT_SERVER: &str = "http://127.0.0.1:5000/";
/// The route the shell probes: `spectrumiq-server`'s health.
pub const HEALTH_PATH: &str = "/api/siq/system/health";
/// The route that names the local console's secret file, to this computer.
pub const LOCAL_PATH: &str = "/api/siq/auth/local";
/// The secret file's name: the shell reads no other.
pub const LOCAL_FILE: &str = "local-login";

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

/// Asks the service at `base` for its health, giving up after `timeout`.
pub async fn probe(base: &Url, timeout: Duration) -> Result<Health, ProbeError> {
    let at = base.to_string();
    let body = get(base, HEALTH_PATH, timeout).await?;
    serde_json::from_str(body.trim())
        .map_err(|error| ProbeError::NotSpectrumIq(at, error.to_string()))
}

/// The local console's secret (ADR-0066): the service names its file to this computer, and the
/// shell, run as the service's account, reads it. Only a file called `local-login` is read, and
/// only 64 hexadecimal digits are given back, so no page can make the shell read another file.
pub async fn local_secret(base: &Url, timeout: Duration) -> Result<String, ProbeError> {
    let at = base.to_string();
    let refused = |why: &str| ProbeError::NotSpectrumIq(at.clone(), why.to_owned());
    let body = get(base, LOCAL_PATH, timeout).await?;
    let named: serde_json::Value =
        serde_json::from_str(body.trim()).map_err(|error| refused(&error.to_string()))?;
    let file = std::path::PathBuf::from(named["file"].as_str().unwrap_or_default());
    if file.file_name().and_then(|name| name.to_str()) != Some(LOCAL_FILE) {
        return Err(refused("the secret's file isn't `local-login`"));
    }
    let secret = std::fs::read_to_string(&file).map_err(|error| refused(&error.to_string()))?;
    let secret = secret.trim();
    if secret.len() != 64 || !secret.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(refused("the secret isn't 64 hexadecimal digits"));
    }
    Ok(secret.to_owned())
}

/// `GET path` of the service at `base`, giving up after `timeout`: the body of a 200. One HTTP/1.1
/// request over loopback, which needs no HTTP client.
async fn get(base: &Url, path: &str, timeout: Duration) -> Result<String, ProbeError> {
    let at = base.to_string();
    let host = base.host_str().unwrap_or("127.0.0.1");
    let address = format!("{host}:{}", base.port_or_known_default().unwrap_or(80));
    let exchange = async {
        let mut stream = TcpStream::connect(&address).await?;
        let request = format!(
            "GET {path} HTTP/1.1\r\nHost: {address}\r\nAccept: application/json\r\n\
             Connection: close\r\n\r\n"
        );
        stream.write_all(request.as_bytes()).await?;
        let mut answer = Vec::new();
        stream.read_to_end(&mut answer).await?;
        Ok::<_, std::io::Error>(answer)
    };
    let answer = match tokio::time::timeout(timeout, exchange).await {
        Ok(Ok(answer)) => answer,
        Ok(Err(_)) | Err(_) => return Err(ProbeError::Unreachable(at)),
    };
    let text = String::from_utf8_lossy(&answer);
    let (head, body) = (text.split_once("\r\n\r\n"))
        .ok_or_else(|| ProbeError::NotSpectrumIq(at.clone(), "no HTTP answer".to_owned()))?;
    let status = head.lines().next().unwrap_or_default();
    if status.split_whitespace().nth(1) != Some("200") {
        return Err(ProbeError::Status(at, status.to_owned()));
    }
    Ok(body.to_owned())
}

#[cfg(test)]
mod tests;
