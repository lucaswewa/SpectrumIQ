//! The accounts: a JSON file of names, roles and argon2id hashes, written whole and replaced, and
//! read again whenever another process, such as the CLI, has changed it.

use std::fmt;
use std::io;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::{Mutex, OnceLock, PoisonError};
use std::time::{Duration, SystemTime};

use argon2::{Argon2, PasswordHasher, PasswordVerifier};
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The fewest characters a password has.
pub const MIN_PASSWORD: usize = 8;

/// What a user may do (architecture §10): each role may do all that the one before it may.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    /// Sees the instrument and its data.
    Viewer,
    /// Also operates it while holding the control token, and runs processing.
    Operator,
    /// Also manages users, deletes data, and takes the token over.
    Admin,
}

impl fmt::Display for Role {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Viewer => "viewer",
            Self::Operator => "operator",
            Self::Admin => "admin",
        })
    }
}

impl FromStr for Role {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, String> {
        match text.to_ascii_lowercase().as_str() {
            "viewer" => Ok(Self::Viewer),
            "operator" => Ok(Self::Operator),
            "admin" => Ok(Self::Admin),
            _ => Err(format!("{text:?} isn't a role: viewer, operator or admin")),
        }
    }
}

/// An account, as shown: its hash stays in the store.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct User {
    /// Its name.
    pub name: String,
    /// What it may do.
    pub role: Role,
    /// When it was made.
    pub created: DateTime<Utc>,
    /// When its password or role last changed.
    pub changed: DateTime<Utc>,
}

/// An account, as stored.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Account {
    #[serde(flatten)]
    user: User,
    /// Its password's argon2id hash, as a PHC string.
    hash: String,
}

/// The file.
#[derive(Debug, Default, Serialize, Deserialize)]
struct Accounts {
    users: Vec<Account>,
}

/// Why the users couldn't be read, written or changed.
#[derive(Debug, thiserror::Error)]
pub enum UserError {
    /// A name with other characters, or too long.
    #[error("{0:?} isn't a user name: 1 to 64 letters, digits, '.', '_' or '-'")]
    Name(String),
    /// A password too short.
    #[error("a password needs at least {MIN_PASSWORD} characters")]
    Password,
    /// A name already taken.
    #[error("there is already a user {0:?}")]
    Exists(String),
    /// A name no user has.
    #[error("there is no user {0:?}")]
    Unknown(String),
    /// The change would leave no admin.
    #[error("the last admin can't be removed or given another role")]
    LastAdmin,
    /// The file couldn't be read or written.
    #[error("the users file {path}: {source}")]
    Io {
        /// The file.
        path: PathBuf,
        /// Why.
        source: io::Error,
    },
    /// The file isn't the store's.
    #[error("the users file {path}: {source}")]
    Format {
        /// The file.
        path: PathBuf,
        /// Why.
        source: serde_json::Error,
    },
    /// Hashing failed: no randomness for the salt, say.
    #[error("hashing the password: {0}")]
    Hash(String),
}

/// The accounts in memory, and when their file was last written as far as they know.
#[derive(Debug)]
struct Loaded {
    accounts: Vec<Account>,
    modified: Option<SystemTime>,
}

/// The users (ADR-0065): a JSON file, by default `users.json` in the service's settings.
#[derive(Debug)]
pub struct UserStore {
    path: PathBuf,
    loaded: Mutex<Loaded>,
}

/// The hash a password is checked against when its name is unknown, so that the time a login
/// takes doesn't tell which names exist.
fn decoy() -> &'static str {
    static DECOY: OnceLock<String> = OnceLock::new();
    DECOY.get_or_init(|| {
        let hash = Argon2::default().hash_password_with_salt(b"no such user", b"spectrumiq-decoy");
        hash.map(|hash| hash.to_string()).unwrap_or_default()
    })
}

fn hash(password: &str) -> Result<String, UserError> {
    if password.chars().count() < MIN_PASSWORD {
        return Err(UserError::Password);
    }
    let hash = Argon2::default().hash_password(password.as_bytes());
    hash.map(|hash| hash.to_string())
        .map_err(|error| UserError::Hash(error.to_string()))
}

fn check_name(name: &str) -> Result<(), UserError> {
    let allowed = |c: char| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-');
    if name.is_empty() || name.len() > 64 || !name.chars().all(allowed) {
        return Err(UserError::Name(name.to_owned()));
    }
    Ok(())
}

impl UserStore {
    /// The store of `path`: its users if the file exists, none until one is added otherwise.
    pub fn open(path: impl Into<PathBuf>) -> Result<Self, UserError> {
        let path = path.into();
        let loaded = read(&path)?;
        Ok(Self {
            path,
            loaded: Mutex::new(loaded),
        })
    }

    /// Its file.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The accounts as they are now, read again first if their file has changed: its time stamp
    /// differs, or is so recent that two writes may share it, file times being kept to the
    /// system's tick of about 15 ms.
    fn current(&self) -> Result<std::sync::MutexGuard<'_, Loaded>, UserError> {
        let mut loaded = self.loaded.lock().unwrap_or_else(PoisonError::into_inner);
        let modified = modified(&self.path);
        let recent = modified
            .and_then(|m| SystemTime::now().duration_since(m).ok())
            .is_none_or(|age| age < Duration::from_secs(2));
        if modified != loaded.modified || (modified.is_some() && recent) {
            *loaded = read(&self.path)?;
        }
        Ok(loaded)
    }

    /// Changes the accounts with `change`, then writes them.
    fn change<T>(
        &self,
        change: impl FnOnce(&mut Vec<Account>) -> Result<T, UserError>,
    ) -> Result<T, UserError> {
        let mut loaded = self.current()?;
        let mut accounts = loaded.accounts.clone();
        let answer = change(&mut accounts)?;
        write(&self.path, &accounts)?;
        *loaded = Loaded {
            accounts,
            modified: modified(&self.path),
        };
        Ok(answer)
    }

    /// The users, by name.
    pub fn list(&self) -> Result<Vec<User>, UserError> {
        let mut users: Vec<User> = (self.current()?.accounts.iter())
            .map(|account| account.user.clone())
            .collect();
        users.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(users)
    }

    /// The user `name`, if there is one.
    pub fn get(&self, name: &str) -> Result<Option<User>, UserError> {
        let loaded = self.current()?;
        let account = loaded.accounts.iter().find(|a| a.user.name == name);
        Ok(account.map(|account| account.user.clone()))
    }

    /// Whether there are no users yet: the first one added should be an admin.
    pub fn is_empty(&self) -> Result<bool, UserError> {
        Ok(self.current()?.accounts.is_empty())
    }

    /// Adds the user `name`, with `password` and `role`.
    pub fn add(&self, name: &str, password: &str, role: Role) -> Result<User, UserError> {
        check_name(name)?;
        let hash = hash(password)?;
        self.change(|accounts| {
            if accounts.iter().any(|a| a.user.name == name) {
                return Err(UserError::Exists(name.to_owned()));
            }
            let now = Utc::now();
            let user = User {
                name: name.to_owned(),
                role,
                created: now,
                changed: now,
            };
            accounts.push(Account {
                user: user.clone(),
                hash,
            });
            Ok(user)
        })
    }

    /// Gives the user `name` the password `password`.
    pub fn set_password(&self, name: &str, password: &str) -> Result<User, UserError> {
        let hash = hash(password)?;
        self.change(|accounts| {
            let account = (accounts.iter_mut())
                .find(|a| a.user.name == name)
                .ok_or_else(|| UserError::Unknown(name.to_owned()))?;
            account.hash = hash;
            account.user.changed = Utc::now();
            Ok(account.user.clone())
        })
    }

    /// Gives the user `name` the role `role`. The last admin keeps theirs.
    pub fn set_role(&self, name: &str, role: Role) -> Result<User, UserError> {
        self.change(|accounts| {
            let admins = accounts
                .iter()
                .filter(|a| a.user.role == Role::Admin)
                .count();
            let account = (accounts.iter_mut())
                .find(|a| a.user.name == name)
                .ok_or_else(|| UserError::Unknown(name.to_owned()))?;
            if account.user.role == Role::Admin && role != Role::Admin && admins == 1 {
                return Err(UserError::LastAdmin);
            }
            account.user.role = role;
            account.user.changed = Utc::now();
            Ok(account.user.clone())
        })
    }

    /// Removes the user `name`. The last admin stays.
    pub fn remove(&self, name: &str) -> Result<User, UserError> {
        self.change(|accounts| {
            let at = (accounts.iter())
                .position(|a| a.user.name == name)
                .ok_or_else(|| UserError::Unknown(name.to_owned()))?;
            let admins = accounts
                .iter()
                .filter(|a| a.user.role == Role::Admin)
                .count();
            if accounts[at].user.role == Role::Admin && admins == 1 {
                return Err(UserError::LastAdmin);
            }
            Ok(accounts.remove(at).user)
        })
    }

    /// The user `name`, if `password` is theirs. An unknown name's password is checked against a
    /// decoy hash, as long as a known one's takes.
    pub fn verify(&self, name: &str, password: &str) -> Result<Option<User>, UserError> {
        let found = {
            let loaded = self.current()?;
            let account = loaded.accounts.iter().find(|a| a.user.name == name);
            account.map(|account| (account.user.clone(), account.hash.clone()))
        };
        let hash = found.as_ref().map_or(decoy(), |(_, hash)| hash.as_str());
        let right = Argon2::default()
            .verify_password(password.as_bytes(), hash)
            .is_ok();
        Ok(found.filter(|_| right).map(|(user, _)| user))
    }
}

/// When `path` was last written: `None` if it doesn't exist.
fn modified(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

fn read(path: &Path) -> Result<Loaded, UserError> {
    let modified = modified(path);
    let text = match std::fs::read(path) {
        Ok(text) => text,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(Loaded {
                accounts: Vec::new(),
                modified: None,
            });
        }
        Err(source) => {
            let path = path.to_owned();
            return Err(UserError::Io { path, source });
        }
    };
    let file: Accounts = serde_json::from_slice(&text).map_err(|source| UserError::Format {
        path: path.to_owned(),
        source,
    })?;
    Ok(Loaded {
        accounts: file.users,
        modified,
    })
}

/// Writes `accounts` to a file beside `path`, then puts it in `path`'s place, so a reader never
/// sees half of it.
fn write(path: &Path, accounts: &[Account]) -> Result<(), UserError> {
    let io = |source| UserError::Io {
        path: path.to_owned(),
        source,
    };
    if let Some(folder) = path.parent() {
        std::fs::create_dir_all(folder).map_err(io)?;
    }
    let file = Accounts {
        users: accounts.to_vec(),
    };
    let text = serde_json::to_vec_pretty(&file).map_err(|source| UserError::Format {
        path: path.to_owned(),
        source,
    })?;
    let next = path.with_extension("json.new");
    std::fs::write(&next, text).map_err(io)?;
    std::fs::rename(&next, path).map_err(io)
}
