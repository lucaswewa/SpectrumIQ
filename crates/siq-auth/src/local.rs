//! The local console's login (ADR-0066): a one-time secret in a file that only the service's own
//! account can open, which the desktop shell, run as that account, reads and exchanges for a
//! session. Each secret works once: the exchange writes the next. A small Windows-API module
//! (ADR-0007): the file's access list holds one entry, its account's, and inherits none.

#![allow(unsafe_code)]

use std::ffi::OsStr;
use std::io;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};

use windows_sys::Win32::Foundation::{CloseHandle, GENERIC_ALL, HANDLE, LocalFree};
use windows_sys::Win32::Security::Authorization::{
    EXPLICIT_ACCESS_W, NO_MULTIPLE_TRUSTEE, SE_FILE_OBJECT, SET_ACCESS, SetEntriesInAclW,
    SetNamedSecurityInfoW, TRUSTEE_IS_SID, TRUSTEE_IS_USER, TRUSTEE_W,
};
use windows_sys::Win32::Security::{
    ACL, DACL_SECURITY_INFORMATION, GetTokenInformation, NO_INHERITANCE,
    PROTECTED_DACL_SECURITY_INFORMATION, TOKEN_QUERY, TOKEN_USER, TokenUser,
};
use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

use crate::sessions::SessionId;

/// The local console's secret, and its file.
#[derive(Debug)]
pub struct LocalLogin {
    path: PathBuf,
    secret: Mutex<String>,
}

impl LocalLogin {
    /// A fresh secret, written to `path` for this process's account alone.
    pub fn create(path: impl Into<PathBuf>) -> io::Result<Self> {
        let path = path.into();
        let secret = fresh()?;
        write_private(&path, &secret)?;
        Ok(Self {
            path,
            secret: Mutex::new(secret),
        })
    }

    /// Its file.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Whether `secret` is the current one. If it is, the next replaces it, in memory and in the
    /// file, so that each is used once.
    pub fn exchange(&self, secret: &str) -> io::Result<bool> {
        let mut current = self.secret.lock().unwrap_or_else(PoisonError::into_inner);
        let (given, expected) = (secret.trim().as_bytes(), current.as_bytes());
        // Compared in constant time, as the length is fixed.
        let same = given.len() == expected.len()
            && given
                .iter()
                .zip(expected)
                .fold(0u8, |d, (a, b)| d | (a ^ b))
                == 0;
        if same {
            let next = fresh()?;
            write_private(&self.path, &next)?;
            *current = next;
        }
        Ok(same)
    }
}

/// A new secret: 256 random bits, as hexadecimal digits.
fn fresh() -> io::Result<String> {
    let id = SessionId::random().map_err(|error| io::Error::other(error.to_string()))?;
    Ok(id.as_str().to_owned())
}

/// Writes `text` to `path` through a file made private first, then put in `path`'s place: no one
/// else can read it at any moment.
fn write_private(path: &Path, text: &str) -> io::Result<()> {
    if let Some(folder) = path.parent() {
        std::fs::create_dir_all(folder)?;
    }
    let next = path.with_extension("new");
    std::fs::write(&next, "")?;
    restrict(&next)?;
    std::fs::write(&next, text)?;
    std::fs::rename(&next, path)
}

/// Gives `path` an access list of one entry, this process's account with full access, and
/// protects it from its folder's inherited entries.
fn restrict(path: &Path) -> io::Result<()> {
    let wide: Vec<u16> = OsStr::new(path).encode_wide().chain([0]).collect();
    let mut token: HANDLE = std::ptr::null_mut();
    // SAFETY: the current process's pseudo-handle, and a handle to fill, closed below.
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let user = token_user(token);
    // SAFETY: the token opened above, closed once.
    unsafe { CloseHandle(token) };
    let user = user?;
    // The buffer starts with a TOKEN_USER, whose SID points into the buffer.
    // SAFETY: GetTokenInformation filled the buffer with a TOKEN_USER, and `user` outlives the
    // pointer's use below.
    let sid = unsafe { (*user.as_ptr().cast::<TOKEN_USER>()).User.Sid };
    let access = EXPLICIT_ACCESS_W {
        grfAccessPermissions: GENERIC_ALL,
        grfAccessMode: SET_ACCESS,
        grfInheritance: NO_INHERITANCE,
        Trustee: TRUSTEE_W {
            pMultipleTrustee: std::ptr::null_mut(),
            MultipleTrusteeOperation: NO_MULTIPLE_TRUSTEE,
            TrusteeForm: TRUSTEE_IS_SID,
            TrusteeType: TRUSTEE_IS_USER,
            ptstrName: sid.cast(),
        },
    };
    let mut acl: *mut ACL = std::ptr::null_mut();
    // SAFETY: one entry, no old list, and a list to fill, freed with LocalFree below.
    let made = unsafe { SetEntriesInAclW(1, &access, std::ptr::null(), &mut acl) };
    if made != 0 {
        return Err(io::Error::from_raw_os_error(made as i32));
    }
    let info = DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION;
    let null = std::ptr::null_mut();
    // SAFETY: a NUL-terminated path, and the list made above; the owner, group and audit list
    // are left as they are.
    let set = unsafe {
        SetNamedSecurityInfoW(
            wide.as_ptr(),
            SE_FILE_OBJECT,
            info,
            null,
            null,
            acl,
            std::ptr::null(),
        )
    };
    // SAFETY: the list SetEntriesInAclW allocated, freed once.
    unsafe { LocalFree(acl.cast()) };
    if set != 0 {
        return Err(io::Error::from_raw_os_error(set as i32));
    }
    Ok(())
}

/// The `TOKEN_USER` of `token`, in a buffer aligned for it.
fn token_user(token: HANDLE) -> io::Result<Vec<u64>> {
    let mut length = 0u32;
    // SAFETY: asking for the size only: no buffer, length 0.
    unsafe { GetTokenInformation(token, TokenUser, std::ptr::null_mut(), 0, &mut length) };
    let mut buffer = vec![0u64; (length as usize).div_ceil(8).max(1)];
    let size = (buffer.len() * 8) as u32;
    // SAFETY: a buffer of `size` bytes, aligned to 8, for the token's user.
    let ok = unsafe {
        GetTokenInformation(
            token,
            TokenUser,
            buffer.as_mut_ptr().cast(),
            size,
            &mut length,
        )
    };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(buffer)
}
