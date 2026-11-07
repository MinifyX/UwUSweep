//! The app list and running uninstallers.

use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, ERROR_ELEVATION_REQUIRED};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JobObjectBasicAccountingInformation,
    QueryInformationJobObject, JOBOBJECT_BASIC_ACCOUNTING_INFORMATION,
};
use windows_sys::Win32::System::Threading::{
    GetExitCodeProcess, ResumeThread, WaitForSingleObject, CREATE_SUSPENDED, INFINITE,
    PROCESS_INFORMATION, STARTUPINFOW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
use winreg::enums::{
    HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, KEY_WOW64_32KEY, KEY_WOW64_64KEY,
};
use winreg::RegKey;

use super::{capture, shell_execute_runas, wide, wp};
use crate::{App, Command, Error, Result, Source};

/// How long to wait for processes an uninstaller left running (NSIS copies
/// itself to %TEMP% and lets the first process end at once).
const JOB_WAIT: Duration = Duration::from_secs(600);

fn hive_key(hive: &str) -> RegKey {
    RegKey::predef(if hive == "HKLM" {
        HKEY_LOCAL_MACHINE
    } else {
        HKEY_CURRENT_USER
    })
}

fn view(wow64: bool) -> u32 {
    if wow64 {
        KEY_WOW64_32KEY
    } else {
        KEY_WOW64_64KEY
    }
}

fn expand(s: String) -> String {
    wp::expand_env(&s, |n| std::env::var(n).ok())
}

fn read_entry(key: &RegKey, hive: &str, key_name: &str, wow64: bool) -> wp::ArpEntry {
    let s = |n: &str| key.get_value::<String, _>(n).ok().map(expand);
    let d = |n: &str| key.get_value::<u32, _>(n).ok();
    wp::ArpEntry {
        hive: hive.to_string(),
        key_name: key_name.to_string(),
        wow64,
        display_name: s("DisplayName"),
        display_version: s("DisplayVersion"),
        publisher: s("Publisher"),
        install_location: s("InstallLocation"),
        estimated_size_kb: d("EstimatedSize"),
        install_date: s("InstallDate"),
        uninstall_string: s("UninstallString"),
        quiet_uninstall_string: s("QuietUninstallString"),
        display_icon: s("DisplayIcon"),
        windows_installer: d("WindowsInstaller") == Some(1),
        system_component: d("SystemComponent") == Some(1),
        parent_key_name: s("ParentKeyName").is_some_and(|p| !p.trim().is_empty()),
        release_type: s("ReleaseType"),
    }
}

fn uninstall_root(hive: &str) -> String {
    let root = if hive == "HKLM" {
        "SOFTWARE"
    } else {
        "Software"
    };
    format!(r"{root}\{}", wp::UNINSTALL_KEY)
}

/// One uninstall entry, read fresh from the registry.
fn read_app(hive: &str, key_name: &str, wow64: bool) -> Option<App> {
    let path = format!(r"{}\{key_name}", uninstall_root(hive));
    let key = hive_key(hive)
        .open_subkey_with_flags(path, KEY_READ | view(wow64))
        .ok()?;
    wp::arp_to_app(&read_entry(&key, hive, key_name, wow64))
}

fn registry_apps() -> Vec<App> {
    let mut apps = Vec::new();
    for (hive, wow64) in [
        ("HKLM", false),
        ("HKLM", true),
        ("HKCU", false),
        ("HKCU", true),
    ] {
        let Ok(list) =
            hive_key(hive).open_subkey_with_flags(uninstall_root(hive), KEY_READ | view(wow64))
        else {
            continue;
        };
        for name in list.enum_keys().flatten() {
            if let Ok(key) = list.open_subkey_with_flags(&name, KEY_READ | view(wow64)) {
                apps.extend(wp::arp_to_app(&read_entry(&key, hive, &name, wow64)));
            }
        }
    }
    wp::merge_duplicates(apps)
}

fn store_apps() -> Vec<App> {
    let argv = wp::powershell_argv(
        "Get-AppxPackage | Where-Object { -not $_.IsFramework -and -not $_.NonRemovable -and \
         $_.SignatureKind -eq 'Store' } | Select-Object Name,PackageFullName,PackageFamilyName,\
         Publisher,Version | ConvertTo-Json -Compress",
    );
    capture(&argv, Duration::from_secs(30))
        .map(|out| {
            wp::parse_appx_json(&out.stdout)
                .iter()
                .filter_map(wp::appx_to_app)
                .collect()
        })
        .unwrap_or_default()
}

pub fn list_apps() -> Result<Vec<App>> {
    let mut apps = registry_apps();
    apps.extend(store_apps());
    // winget is optional: missing, slow on its first run or failing, the list
    // simply has no winget ids.
    let argv = [
        "winget",
        "list",
        "--accept-source-agreements",
        "--disable-interactivity",
    ]
    .map(String::from);
    if let Some(out) = capture(&argv, Duration::from_secs(20)) {
        wp::match_winget(&mut apps, &wp::parse_winget_list(&out.stdout));
    }
    apps.sort_by_cached_key(|a| a.name.to_lowercase());
    Ok(apps)
}

/// Runs an uninstall command line and waits for it and everything it
/// started.
fn run_line(line: &str) -> Result<u32> {
    let mut cmd = wide(line);
    // SAFETY: zeroed STARTUPINFOW/PROCESS_INFORMATION are their documented
    // empty states; every handle received is closed below.
    unsafe {
        let mut si: STARTUPINFOW = std::mem::zeroed();
        si.cb = std::mem::size_of::<STARTUPINFOW>() as u32;
        let mut pi: PROCESS_INFORMATION = std::mem::zeroed();
        let ok = windows_sys::Win32::System::Threading::CreateProcessW(
            std::ptr::null(),
            cmd.as_mut_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            0,
            CREATE_SUSPENDED,
            std::ptr::null(),
            std::ptr::null(),
            &si,
            &mut pi,
        );
        if ok == 0 {
            let e = GetLastError();
            if e == ERROR_ELEVATION_REQUIRED {
                // Elevated processes cannot join our job; wait for the one.
                let (file, params) = wp::split_command_line(line);
                return shell_execute_runas(&file, &params, SW_SHOWNORMAL);
            }
            return Err(std::io::Error::from_raw_os_error(e as i32).into());
        }
        // Started suspended so it cannot spawn anything before it is in the job.
        let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
        let in_job = !job.is_null() && AssignProcessToJobObject(job, pi.hProcess) != 0;
        ResumeThread(pi.hThread);
        CloseHandle(pi.hThread);
        WaitForSingleObject(pi.hProcess, INFINITE);
        let mut code = 0;
        GetExitCodeProcess(pi.hProcess, &mut code);
        CloseHandle(pi.hProcess);
        if in_job {
            let start = Instant::now();
            while start.elapsed() < JOB_WAIT {
                let mut info: JOBOBJECT_BASIC_ACCOUNTING_INFORMATION = std::mem::zeroed();
                let ok = QueryInformationJobObject(
                    job,
                    JobObjectBasicAccountingInformation,
                    &mut info as *mut _ as *mut _,
                    std::mem::size_of::<JOBOBJECT_BASIC_ACCOUNTING_INFORMATION>() as u32,
                    std::ptr::null_mut(),
                );
                if ok == 0 || info.ActiveProcesses == 0 {
                    break;
                }
                std::thread::sleep(Duration::from_millis(500));
            }
        }
        if !job.is_null() {
            CloseHandle(job);
        }
        Ok(code)
    }
}

fn exit_result(code: u32) -> Result<()> {
    match code {
        // 3010 and 1641: done, a restart finishes it.
        0 | 3010 | 1641 => Ok(()),
        1602 | 1223 => Err(Error::Cancelled),
        c => Err(Error::Failed(format!(
            "The uninstaller ended with code {c}."
        ))),
    }
}

pub fn uninstall(app: &App, quiet: bool) -> Result<()> {
    match &app.source {
        Source::Registry {
            hive, key, wow64, ..
        } => {
            // The command comes from the registry again, not from the app
            // passed in, which the elevated helper got from a job file.
            let key_name = key.rsplit('\\').next().unwrap_or_default();
            if !matches!(hive.as_str(), "HKLM" | "HKCU") || key_name.is_empty() {
                return Err(Error::Failed("Not an uninstall entry.".into()));
            }
            let fresh = read_app(hive, key_name, *wow64).ok_or_else(|| {
                Error::Failed("This program is no longer in the list of installed programs.".into())
            })?;
            let line = quiet
                .then(|| fresh.quiet_uninstall.clone())
                .flatten()
                .or(fresh.uninstall.clone());
            match (line, &app.winget_id) {
                (Some(Command::Line { line }), _) => exit_result(run_line(&line)?),
                (None, Some(id)) if !id.starts_with('-') => {
                    let mut argv: Vec<String> = [
                        "winget",
                        "uninstall",
                        "--id",
                        id,
                        "--exact",
                        "--disable-interactivity",
                    ]
                    .map(String::from)
                    .to_vec();
                    if quiet {
                        argv.push("--silent".into());
                    }
                    let out = capture(&argv, Duration::from_secs(1800))
                        .ok_or_else(|| Error::Failed("winget did not finish.".into()))?;
                    match out.code {
                        Some(0) => Ok(()),
                        c => Err(Error::Failed(format!("winget ended with code {c:?}."))),
                    }
                }
                _ => Err(Error::Failed("This program has no uninstaller.".into())),
            }
        }
        Source::Store {
            package_full_name, ..
        } => {
            if !wp::valid_package_full_name(package_full_name) {
                return Err(Error::Failed("Not a valid package name.".into()));
            }
            let out = capture(
                &wp::remove_appx_argv(package_full_name),
                Duration::from_secs(1800),
            )
            .ok_or_else(|| Error::Failed("Removing the package did not finish.".into()))?;
            if out.code == Some(0) {
                Ok(())
            } else {
                let detail = out.stderr.lines().map(str::trim).find(|l| !l.is_empty());
                Err(Error::Failed(
                    detail.unwrap_or("Removing the package failed.").to_string(),
                ))
            }
        }
        _ => Err(Error::Unsupported),
    }
}
