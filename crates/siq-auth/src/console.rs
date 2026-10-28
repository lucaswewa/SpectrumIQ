//! A password read at the console without showing it: a small Windows-API module (ADR-0007),
//! turning the console's echo off while the line is read.

#![allow(unsafe_code)]

use std::io::{self, BufRead, Write};

use windows_sys::Win32::System::Console::{
    CONSOLE_MODE, ENABLE_ECHO_INPUT, GetConsoleMode, GetStdHandle, STD_INPUT_HANDLE, SetConsoleMode,
};

/// A line from standard input, after `prompt` on standard error: not shown as it is typed when
/// the input is a console, read as it comes when it is a pipe, as a script's is.
pub fn read_password(prompt: &str) -> io::Result<String> {
    eprint!("{prompt}");
    io::stderr().flush()?;
    // SAFETY: a standard handle of this process, which needn't be closed.
    let input = unsafe { GetStdHandle(STD_INPUT_HANDLE) };
    let mut mode: CONSOLE_MODE = 0;
    // SAFETY: the handle, and a mode to fill; it fails for a handle that isn't a console's.
    let console = unsafe { GetConsoleMode(input, &mut mode) } != 0;
    if console {
        // SAFETY: the same console handle, with its own mode less the echo.
        unsafe { SetConsoleMode(input, mode & !ENABLE_ECHO_INPUT) };
    }
    let mut line = String::new();
    let read = io::stdin().lock().read_line(&mut line);
    if console {
        // SAFETY: the same handle, given back the mode it had.
        unsafe { SetConsoleMode(input, mode) };
        eprintln!();
    }
    read?;
    Ok(line.trim_end_matches(['\r', '\n']).to_owned())
}
