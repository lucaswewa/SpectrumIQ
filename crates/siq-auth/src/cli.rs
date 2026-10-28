//! The `spectrumiq-server user` commands, apart from their parsing, so that a script, or the
//! `user_cli` example, runs what the CLI runs. While there is no admin, the user added is one: the
//! first admin manages the rest.

use std::io::{self, Write};

use crate::users::{Role, UserError, UserStore};

/// A command on the users.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UserCommand {
    /// Adds a user: an admin while there is none, a viewer otherwise, if no role is given.
    Add {
        /// Its name.
        name: String,
        /// Its role.
        role: Option<Role>,
    },
    /// Changes a user's password.
    Passwd {
        /// Its name.
        name: String,
    },
    /// Changes a user's role.
    Role {
        /// Its name.
        name: String,
        /// Its new role.
        role: Role,
    },
    /// Lists the users.
    List,
    /// Removes a user.
    Remove {
        /// Its name.
        name: String,
    },
}

/// Why a command failed.
#[derive(Debug, thiserror::Error)]
pub enum CliError {
    /// The store refused it.
    #[error(transparent)]
    User(#[from] UserError),
    /// A password couldn't be read, or the answer written.
    #[error(transparent)]
    Io(#[from] io::Error),
    /// The password and its repetition differ.
    #[error("the passwords differ")]
    Mismatch,
    /// There is no admin, and the user wouldn't be one: no one could manage the users.
    #[error("there is no admin yet, and the first must be one, to manage the rest: --role admin")]
    FirstAdmin,
}

/// A new password for `name`, asked twice of `ask`.
fn new_password(
    name: &str,
    ask: &mut dyn FnMut(&str) -> io::Result<String>,
) -> Result<String, CliError> {
    let password = ask(&format!("Password for {name}: "))?;
    if ask("The same again: ")? != password {
        return Err(CliError::Mismatch);
    }
    Ok(password)
}

/// Runs `command` on `store`, asking `ask` for passwords by their prompts, and writing what it
/// did to `out`.
pub fn run(
    store: &UserStore,
    command: &UserCommand,
    ask: &mut dyn FnMut(&str) -> io::Result<String>,
    out: &mut dyn Write,
) -> Result<(), CliError> {
    match command {
        UserCommand::Add { name, role } => {
            let first = !store.list()?.iter().any(|user| user.role == Role::Admin);
            let role = match (role, first) {
                (Some(role), true) if *role != Role::Admin => return Err(CliError::FirstAdmin),
                (Some(role), _) => *role,
                (None, true) => Role::Admin,
                (None, false) => Role::Viewer,
            };
            let password = new_password(name, ask)?;
            let user = store.add(name, &password, role)?;
            writeln!(out, "added {} as {}", user.name, user.role)?;
        }
        UserCommand::Passwd { name } => {
            if store.get(name)?.is_none() {
                return Err(UserError::Unknown(name.clone()).into());
            }
            let password = new_password(name, ask)?;
            store.set_password(name, &password)?;
            writeln!(out, "changed {name}'s password")?;
        }
        UserCommand::Role { name, role } => {
            let user = store.set_role(name, *role)?;
            writeln!(out, "{} is now {}", user.name, user.role)?;
        }
        UserCommand::List => {
            let users = store.list()?;
            if users.is_empty() {
                writeln!(out, "no users in {}", store.path().display())?;
            }
            for user in users {
                let created = user.created.format("%Y-%m-%d");
                writeln!(out, "{:<24} {:<9} since {created}", user.name, user.role)?;
            }
        }
        UserCommand::Remove { name } => {
            let user = store.remove(name)?;
            writeln!(out, "removed {}", user.name)?;
        }
    }
    Ok(())
}
