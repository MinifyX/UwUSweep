//! Windows: the uninstall registry, Store packages and winget for the app
//! list, the registry for leftovers and autostart, UAC for admin rights.

mod apps;
mod autostart;
mod registry;
mod secure;

use std::ffi::OsString;
use std::io::Read;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::mpsc;
use std::time::Duration;

use windows_sys::core::GUID;
use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, ERROR_CANCELLED};
use windows_sys::Win32::Security::{
    GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY,
};
use windows_sys::Win32::System::Com::CoTaskMemFree;
use windows_sys::Win32::System::Threading::{
    GetCurrentProcess, GetExitCodeProcess, OpenProcessToken, WaitForSingleObject, CREATE_NO_WINDOW,
    INFINITE,
};
use windows_sys::Win32::UI::Shell::{
    FOLDERID_CommonPrograms, FOLDERID_Desktop, FOLDERID_LocalAppData, FOLDERID_LocalAppDataLow,
    FOLDERID_Profile, FOLDERID_ProgramData, FOLDERID_ProgramFiles, FOLDERID_ProgramFilesX86,
    FOLDERID_Programs, FOLDERID_PublicDesktop, FOLDERID_RoamingAppData, FOLDERID_UserProgramFiles,
    SHGetKnownFolderPath, ShellExecuteExW, SEE_MASK_FLAG_NO_UI, SEE_MASK_NOASYNC,
    SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::SW_HIDE;

pub use apps::{list_apps, uninstall};
pub use autostart::{autostart_list, autostart_set};
pub use registry::{quarantine_registry, registry_leftovers, registry_snapshot, restore_registry};
pub use secure::{admin_quarantine_root, secure_dir};

use super::windows_parse as wp;
use crate::scan::Root;
use crate::{Error, Result, ScanLevel};

/// A NUL-terminated UTF-16 string for the Win32 API.
fn wide(s: impl AsRef<std::ffi::OsStr>) -> Vec<u16> {
    s.as_ref().encode_wide().chain([0]).collect()
}

fn known_folder(id: &GUID) -> Option<PathBuf> {
    let mut p = std::ptr::null_mut();
    // SAFETY: the API allocates `p`, which is freed below whatever it returns.
    unsafe {
        let hr = SHGetKnownFolderPath(id, 0, std::ptr::null_mut(), &mut p);
        let path = (hr >= 0 && !p.is_null()).then(|| {
            let len = (0..).take_while(|&i| *p.add(i) != 0).count();
            PathBuf::from(OsString::from_wide(std::slice::from_raw_parts(p, len)))
        });
        CoTaskMemFree(p as *const _);
        path
    }
}

struct Captured {
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

/// Runs a console program without a window and collects its output; `None`
/// when it cannot start or takes longer than `timeout` (it is killed then).
fn capture(argv: &[String], timeout: Duration) -> Option<Captured> {
    let mut child = std::process::Command::new(argv.first()?)
        .args(&argv[1..])
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .ok()?;
    let mut out = child.stdout.take()?;
    let mut err = child.stderr.take()?;
    let errs = std::thread::spawn(move || {
        let mut b = Vec::new();
        let _ = err.read_to_end(&mut b);
        b
    });
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut b = Vec::new();
        let _ = out.read_to_end(&mut b);
        let _ = tx.send(b);
    });
    match rx.recv_timeout(timeout) {
        Ok(stdout) => {
            let code = child.wait().ok().and_then(|s| s.code());
            let stderr = errs.join().unwrap_or_default();
            Some(Captured {
                code,
                stdout: String::from_utf8_lossy(&stdout).into_owned(),
                stderr: String::from_utf8_lossy(&stderr).into_owned(),
            })
        }
        Err(_) => {
            let _ = child.kill();
            let _ = child.wait();
            None
        }
    }
}

/// Starts `file` with the UAC prompt and waits for it.
fn shell_execute_runas(file: &str, params: &str, show: i32) -> Result<u32> {
    let (verb, file, params) = (wide("runas"), wide(file), wide(params));
    // SAFETY: plain struct of pointers and integers; zero is its documented
    // empty state, and the strings outlive the call.
    unsafe {
        let mut info: SHELLEXECUTEINFOW = std::mem::zeroed();
        info.cbSize = std::mem::size_of::<SHELLEXECUTEINFOW>() as u32;
        info.fMask = SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC | SEE_MASK_FLAG_NO_UI;
        info.lpVerb = verb.as_ptr();
        info.lpFile = file.as_ptr();
        info.lpParameters = params.as_ptr();
        info.nShow = show;
        if ShellExecuteExW(&mut info) == 0 {
            let e = GetLastError();
            return Err(if e == ERROR_CANCELLED {
                Error::Cancelled
            } else {
                std::io::Error::from_raw_os_error(e as i32).into()
            });
        }
        if info.hProcess.is_null() {
            return Err(Error::Failed(
                "The program started without a process to wait for.".into(),
            ));
        }
        WaitForSingleObject(info.hProcess, INFINITE);
        let mut code = 0;
        GetExitCodeProcess(info.hProcess, &mut code);
        CloseHandle(info.hProcess);
        Ok(code)
    }
}

pub fn is_elevated() -> bool {
    // SAFETY: the token handle is closed again; the buffer is the size given.
    unsafe {
        let mut token = std::ptr::null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return false;
        }
        let mut elevation = TOKEN_ELEVATION { TokenIsElevated: 0 };
        let mut len = 0;
        let ok = GetTokenInformation(
            token,
            TokenElevation,
            &mut elevation as *mut _ as *mut _,
            std::mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut len,
        );
        CloseHandle(token);
        ok != 0 && elevation.TokenIsElevated != 0
    }
}

pub fn run_elevated(exe: &Path, args: &[String]) -> Result<i32> {
    shell_execute_runas(&exe.to_string_lossy(), &wp::join_args(args), SW_HIDE).map(|c| c as i32)
}

pub fn leftover_roots(level: ScanLevel) -> Vec<Root> {
    let deep = level == ScanLevel::Deep;
    let d = if deep { 2 } else { 1 };
    let mut list = vec![
        (FOLDERID_ProgramFiles, d, true),
        (FOLDERID_ProgramFilesX86, d, true),
        (FOLDERID_ProgramData, 1, true),
        (FOLDERID_RoamingAppData, d, false),
        (FOLDERID_LocalAppData, d, false),
        (FOLDERID_UserProgramFiles, 1, false),
        (FOLDERID_Programs, 2, false),
        (FOLDERID_CommonPrograms, 2, true),
    ];
    if deep {
        list.extend([
            (FOLDERID_LocalAppDataLow, 1, false),
            (FOLDERID_Profile, 1, false),
            (FOLDERID_Desktop, 1, false),
            (FOLDERID_PublicDesktop, 1, true),
        ]);
    }
    let mut roots: Vec<Root> = Vec::new();
    for (id, depth, needs_admin) in list {
        if let Some(path) = known_folder(&id) {
            // Program Files (x86) is Program Files on 32-bit Windows.
            if !roots.iter().any(|r| r.path == path) {
                roots.push(Root {
                    path,
                    depth,
                    needs_admin,
                });
            }
        }
    }
    roots
}

pub fn create_restore_point(description: &str) -> Result<()> {
    let d = wp::restore_point_description(description);
    let argv = wp::powershell_argv(&format!(
        "Checkpoint-Computer -Description '{d}' -RestorePointType APPLICATION_UNINSTALL"
    ));
    // Creating one can take minutes on a busy disk.
    let out = capture(&argv, Duration::from_secs(600))
        .ok_or_else(|| Error::Failed("Creating the restore point did not finish.".into()))?;
    // Windows makes at most one per day and only warns about it; that is fine.
    if out.code == Some(0) {
        return Ok(());
    }
    let detail = out
        .stderr
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("");
    Err(Error::Failed(format!(
        "Could not create a restore point. Is System Protection turned on for the system drive? {detail}"
    )))
}
