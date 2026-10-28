//! The audit trail (ADR-0067): who did what to the instrument and its data, as JSON Lines, a file a
//! month, `YYYY-MM.jsonl`. Each line's `hash` is the SHA-256 of the line without it, which holds
//! the line before's as `prev_hash`; so an edit, a deletion or a reordering breaks the chain, and
//! [`verify`] says where. A month's first line chains to the last month's last.
//!
//! [`AuditLog::record`] doesn't wait: a thread of the trail's own numbers, chains and writes each
//! entry, to disk before the next.

use std::fs::{File, OpenOptions};
use std::io::{self, BufRead, BufReader, Write};
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::JoinHandle;

use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use siq_types::Clock;

/// The `prev_hash` of the very first entry.
pub const GENESIS: &str = "0000000000000000000000000000000000000000000000000000000000000000";

/// What happened.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// A login, with a password or the local console's secret.
    Login,
    /// A login with a wrong name or password.
    LoginFailed,
    /// A login refused after too many failures.
    LoginRefused,
    /// A logout.
    Logout,
    /// The control token changed hands.
    Token,
    /// A request that would change something: its route, its parameters, and its answer.
    Request,
    /// Such a request refused: its origin, its caller's role or a missing token.
    Refused,
    /// A job queued, started or ended.
    Job,
    /// A workflow's run started or ended.
    Workflow,
    /// A dataset deleted.
    DatasetDeleted,
}

/// An entry, as recorded: what happened, and who did it from where.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Event {
    /// What happened.
    pub kind: Kind,
    /// The user, if known.
    pub user: Option<String>,
    /// The session, by its first 8 digits: a full identifier is a credential.
    pub session: Option<String>,
    /// The client's address, if known.
    pub address: Option<IpAddr>,
    /// The rest: the route, the parameters, the outcome.
    pub detail: Value,
}

impl Event {
    /// An event of `kind`, its detail `detail`, of no one in particular.
    pub fn new(kind: Kind, detail: Value) -> Self {
        Self {
            kind,
            user: None,
            session: None,
            address: None,
            detail,
        }
    }

    /// The event as `user`'s, of `session` (an identifier, shortened here), from `address`.
    pub fn by(mut self, user: &str, session: Option<&str>, address: Option<IpAddr>) -> Self {
        self.user = Some(user.to_owned());
        self.session = session.map(|id| id.chars().take(8).collect());
        self.address = address;
        self
    }
}

/// A line as written, without its hash: the field order is the hash's.
#[derive(Serialize)]
struct Line<'a> {
    seq: u64,
    time: DateTime<Utc>,
    #[serde(flatten)]
    event: &'a Event,
    prev_hash: &'a str,
}

/// What the writer is asked to do.
enum Order {
    Record(DateTime<Utc>, Box<Event>),
    Flush(mpsc::Sender<()>),
}

/// The chain's end, as the writer keeps it.
#[derive(Debug)]
struct Head {
    seq: u64,
    hash: String,
}

/// The audit trail (ADR-0067): the folder of its monthly files, and its writer.
#[derive(Debug)]
pub struct AuditLog {
    folder: PathBuf,
    clock: Arc<dyn Clock>,
    orders: Mutex<Option<mpsc::Sender<Order>>>,
    writer: Mutex<Option<JoinHandle<()>>>,
}

impl AuditLog {
    /// The trail in `folder`, continued from its last entry, its entries timed by `clock`.
    pub fn open(folder: impl Into<PathBuf>, clock: Arc<dyn Clock>) -> io::Result<Self> {
        let folder = folder.into();
        std::fs::create_dir_all(&folder)?;
        let mut head = last(&folder)?;
        let (orders, inbox) = mpsc::channel::<Order>();
        let root = folder.clone();
        let writer = std::thread::Builder::new()
            .name("audit".to_owned())
            .spawn(move || {
                let mut open: Option<(String, File)> = None;
                for order in inbox {
                    match order {
                        Order::Record(now, event) => {
                            if let Err(error) = append(&root, &mut open, &mut head, now, &event) {
                                tracing::error!(%error, "the audit trail couldn't be written");
                            }
                        }
                        Order::Flush(done) => {
                            let _ = done.send(());
                        }
                    }
                }
            })?;
        Ok(Self {
            folder,
            clock,
            orders: Mutex::new(Some(orders)),
            writer: Mutex::new(Some(writer)),
        })
    }

    /// Its folder.
    pub fn folder(&self) -> &Path {
        &self.folder
    }

    /// Records `event`, timed now: written at once if the writer is free, after the entries
    /// before it otherwise.
    pub fn record(&self, event: Event) {
        let now = self.clock.now_utc();
        let orders = self.orders.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(orders) = orders.as_ref() {
            let _ = orders.send(Order::Record(now, Box::new(event)));
        }
    }

    /// Waits until every entry recorded so far is on disk.
    pub fn flush(&self) {
        let (done, wait) = mpsc::channel();
        let sent = {
            let orders = self.orders.lock().unwrap_or_else(PoisonError::into_inner);
            (orders.as_ref()).is_some_and(|orders| orders.send(Order::Flush(done)).is_ok())
        };
        if sent {
            let _ = wait.recv();
        }
    }

    /// The entries that `filter` takes, newest first, `offset` and `limit` of them; and how many
    /// there are in all.
    pub fn query(&self, filter: &Filter) -> io::Result<Page> {
        self.flush();
        let mut taken = Vec::new();
        for path in files(&self.folder)?.into_iter().rev() {
            let mut month = Vec::new();
            for line in BufReader::new(File::open(&path)?).lines() {
                let Ok(entry) = serde_json::from_str::<Value>(&line?) else {
                    continue;
                };
                if filter.takes(&entry) {
                    month.push(entry);
                }
            }
            taken.extend(month.into_iter().rev());
        }
        let total = taken.len();
        let limit = if filter.limit == 0 { 100 } else { filter.limit };
        let entries = taken.into_iter().skip(filter.offset).take(limit).collect();
        Ok(Page { entries, total })
    }
}

impl Drop for AuditLog {
    /// Lets the writer finish what it was given, then waits for it.
    fn drop(&mut self) {
        drop(
            self.orders
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .take(),
        );
        if let Some(writer) = self
            .writer
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
        {
            let _ = writer.join();
        }
    }
}

/// Which entries a query takes.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(default)]
pub struct Filter {
    /// From this time on, if given.
    pub from: Option<DateTime<Utc>>,
    /// Before this time, if given.
    pub to: Option<DateTime<Utc>>,
    /// This user's only, if given.
    pub user: Option<String>,
    /// This kind only, if given.
    pub kind: Option<Kind>,
    /// The entries skipped, newest first.
    pub offset: usize,
    /// The most given back: 100 if 0.
    #[serde(default)]
    pub limit: usize,
}

impl Filter {
    fn takes(&self, entry: &Value) -> bool {
        let time = (entry["time"].as_str()).and_then(|t| t.parse::<DateTime<Utc>>().ok());
        let kind = serde_json::from_value::<Kind>(entry["kind"].clone()).ok();
        self.from.is_none_or(|from| time.is_some_and(|t| t >= from))
            && self.to.is_none_or(|to| time.is_some_and(|t| t < to))
            && (self.user.as_deref()).is_none_or(|user| entry["user"].as_str() == Some(user))
            && self.kind.is_none_or(|k| kind == Some(k))
    }
}

/// A page of entries.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct Page {
    /// The entries, newest first, as written.
    pub entries: Vec<Value>,
    /// How many the filter takes in all.
    pub total: usize,
}

/// The trail's files, oldest first: `YYYY-MM.jsonl`.
fn files(folder: &Path) -> io::Result<Vec<PathBuf>> {
    let mut found: Vec<PathBuf> = std::fs::read_dir(folder)?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| {
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default();
            name.len() == "YYYY-MM.jsonl".len() && name.ends_with(".jsonl")
        })
        .collect();
    found.sort();
    Ok(found)
}

/// The hash in a line, and the text it was taken over: the line without `,"hash":"…"`.
fn split(line: &str) -> Option<(String, &str)> {
    let at = line.rfind(",\"hash\":\"")?;
    let hash = line[at..]
        .strip_prefix(",\"hash\":\"")?
        .strip_suffix("\"}")?;
    (hash.len() == 64).then(|| (format!("{}}}", &line[..at]), hash))
}

fn sha256(text: &str) -> String {
    Sha256::digest(text.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// The chain's end in `folder`: its last entry's number and hash, or the genesis.
fn last(folder: &Path) -> io::Result<Head> {
    let mut head = Head {
        seq: 0,
        hash: GENESIS.to_owned(),
    };
    if let Some(path) = files(folder)?.pop() {
        for line in BufReader::new(File::open(path)?).lines() {
            let line = line?;
            let seq = serde_json::from_str::<Value>(&line)
                .ok()
                .and_then(|entry| entry["seq"].as_u64());
            if let (Some(seq), Some((_, hash))) = (seq, split(&line)) {
                head = Head {
                    seq,
                    hash: hash.to_owned(),
                };
            }
        }
    }
    Ok(head)
}

/// Writes `event` as the chain's next line, in `now`'s month's file, synced to disk.
fn append(
    folder: &Path,
    open: &mut Option<(String, File)>,
    head: &mut Head,
    now: DateTime<Utc>,
    event: &Event,
) -> io::Result<()> {
    let month = now.format("%Y-%m").to_string();
    if open.as_ref().is_none_or(|(m, _)| *m != month) {
        let path = folder.join(format!("{month}.jsonl"));
        let file = OpenOptions::new().create(true).append(true).open(path)?;
        *open = Some((month, file));
    }
    let Some((_, file)) = open.as_mut() else {
        return Ok(());
    };
    let line = Line {
        seq: head.seq + 1,
        time: now,
        event,
        prev_hash: &head.hash,
    };
    let text = serde_json::to_string(&line).map_err(io::Error::other)?;
    let hash = sha256(&text);
    let written = format!("{},\"hash\":\"{hash}\"}}\n", &text[..text.len() - 1]);
    file.write_all(written.as_bytes())?;
    file.sync_data()?;
    *head = Head {
        seq: head.seq + 1,
        hash,
    };
    Ok(())
}

/// What a verification found: the entries and files of an intact chain, and its last hash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Verified {
    /// The entries.
    pub entries: u64,
    /// The files.
    pub files: usize,
    /// The last entry's hash: kept elsewhere, it shows a truncation too.
    pub head: String,
}

/// Where the chain breaks.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{file}, line {line}: {why}")]
pub struct Broken {
    /// The file.
    pub file: String,
    /// The line, from 1.
    pub line: usize,
    /// What is wrong.
    pub why: String,
}

/// Checks the trail in `folder`, file after file: every line hashes to its `hash`, holds the one
/// before's as its `prev_hash`, and is numbered one after it.
pub fn verify(folder: &Path) -> Result<Verified, Broken> {
    let broken = |file: &Path, line: usize, why: String| Broken {
        file: file.display().to_string(),
        line,
        why,
    };
    let mut head = Head {
        seq: 0,
        hash: GENESIS.to_owned(),
    };
    let paths = files(folder).map_err(|error| broken(folder, 0, error.to_string()))?;
    for path in &paths {
        let file = File::open(path).map_err(|error| broken(path, 0, error.to_string()))?;
        for (n, line) in BufReader::new(file).lines().enumerate() {
            let line = line.map_err(|error| broken(path, n + 1, error.to_string()))?;
            let Some((text, hash)) = split(&line) else {
                return Err(broken(path, n + 1, "no hash at its end".into()));
            };
            if sha256(&text) != hash {
                return Err(broken(
                    path,
                    n + 1,
                    "its hash isn't its text's: edited".into(),
                ));
            }
            let entry: Value = serde_json::from_str(&line)
                .map_err(|error| broken(path, n + 1, error.to_string()))?;
            if entry["prev_hash"].as_str() != Some(head.hash.as_str()) {
                let why = "it doesn't follow the line before: one removed or moved";
                return Err(broken(path, n + 1, why.into()));
            }
            if entry["seq"].as_u64() != Some(head.seq + 1) {
                return Err(broken(
                    path,
                    n + 1,
                    format!("it isn't entry {}", head.seq + 1),
                ));
            }
            head = Head {
                seq: head.seq + 1,
                hash: hash.to_owned(),
            };
        }
    }
    Ok(Verified {
        entries: head.seq,
        files: paths.len(),
        head: head.hash,
    })
}
