//! A scripted session of the user commands (ADR-0065), as `spectrumiq-server user …` runs them: the
//! first admin, then an operator and a viewer, a role and a password changed, a removal, and the
//! refusals of the last admin's removal and of a short password; then a login, a session, and
//! failed logins past the limit.
//!
//! ```text
//! cargo run -p siq-auth --example user_cli [-- --keep <folder>]
//! ```
//!
//! The users go in a temporary folder removed at the end, or in `<folder>/users.json` with
//! `--keep`.

use std::error::Error;
use std::io;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use siq_auth::cli::{self, UserCommand};
use siq_auth::{LoginLimiter, Role, SessionPolicy, SessionStore, USERS_FILE, UserStore};
use siq_types::SystemClock;

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = std::env::args().collect();
    let kept = (args.iter().position(|arg| arg == "--keep")).and_then(|at| args.get(at + 1));
    let temporary = tempfile::tempdir()?;
    let folder = kept.map_or_else(|| temporary.path().to_owned(), PathBuf::from);
    let store = UserStore::open(folder.join(USERS_FILE))?;
    println!("users in {}", store.path().display());

    // Each password is answered twice, as the CLI asks; the script's answers in order.
    let mut answers = [
        "correct horse",
        "correct horse",
        "operator seven",
        "operator seven",
        "viewer eight",
        "viewer eight",
        "a new password",
        "a new password",
        "short",
        "short",
    ]
    .into_iter();
    let mut ask = |prompt: &str| -> io::Result<String> {
        let answer = answers.next().unwrap_or_default();
        println!("  {prompt}{}", "*".repeat(answer.len()));
        Ok(answer.to_owned())
    };
    let name = |name: &str| name.to_owned();
    let commands = [
        UserCommand::Add {
            name: name("ada"),
            role: None,
        },
        UserCommand::Add {
            name: name("olga"),
            role: Some(Role::Operator),
        },
        UserCommand::Add {
            name: name("vic"),
            role: None,
        },
        UserCommand::Role {
            name: name("vic"),
            role: Role::Operator,
        },
        UserCommand::Passwd { name: name("olga") },
        UserCommand::Remove { name: name("vic") },
        UserCommand::Remove { name: name("ada") },
        UserCommand::Add {
            name: name("sam"),
            role: None,
        },
        UserCommand::List,
    ];
    for command in &commands {
        println!("> {command:?}");
        if let Err(error) = cli::run(&store, command, &mut ask, &mut io::stdout()) {
            println!("refused: {error}");
        }
    }

    // A login: the password checked, a session made; then failures past the limit.
    let clock = Arc::new(SystemClock::new());
    let sessions = SessionStore::new(clock.clone(), SessionPolicy::default());
    let limiter = LoginLimiter::new(clock, 5, Duration::from_secs(300));
    let olga = store
        .verify("olga", "a new password")?
        .ok_or("olga's password")?;
    let session = sessions.create(&olga, None)?;
    println!(
        "olga logged in as {}: {:?}, until {}",
        session.role,
        session.id,
        sessions.expires(&session).format("%H:%M UTC")
    );
    for attempt in 1..=6 {
        if let Some(wait) = limiter.wait("olga", None) {
            println!("attempt {attempt} refused: wait {} s", wait.as_secs());
            break;
        }
        let right = store.verify("olga", "operator seven")?.is_some();
        limiter.failed("olga", None);
        println!(
            "attempt {attempt}: the old password, {}",
            if right { "right" } else { "wrong" }
        );
    }
    Ok(())
}
