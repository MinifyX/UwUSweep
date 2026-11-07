//! Run keys and Startup folders, switched on and off like Task Manager does:
//! through `Explorer\StartupApproved`, leaving the entry itself alone.

use std::path::Path;

use windows_sys::Win32::UI::Shell::{FOLDERID_CommonStartup, FOLDERID_Startup};
use winreg::enums::*;
use winreg::types::FromRegValue;

use super::registry::{hive_of, open};
use super::{known_folder, wp};
use crate::{AutostartEntry, AutostartPlace, Error, Result};

const APPROVED: &str = r"Microsoft\Windows\CurrentVersion\Explorer\StartupApproved";

/// (place, id prefix, hive, key below the hive, StartupApproved subkey)
const RUN: [(AutostartPlace, &str, &str, &str, &str); 3] = [
    (
        AutostartPlace::RunUser,
        "run:HKCU",
        "HKCU",
        r"Software\Microsoft\Windows\CurrentVersion\Run",
        "Run",
    ),
    (
        AutostartPlace::RunMachine,
        "run:HKLM",
        "HKLM",
        r"SOFTWARE\Microsoft\Windows\CurrentVersion\Run",
        "Run",
    ),
    (
        AutostartPlace::RunMachine,
        "run32:HKLM",
        "HKLM",
        r"SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Run",
        "Run32",
    ),
];

fn approved_path(hive: &str, sub: &str) -> String {
    let root = if hive == "HKLM" {
        "SOFTWARE"
    } else {
        "Software"
    };
    format!(r"{hive}\{root}\{APPROVED}\{sub}")
}

fn is_enabled(hive: &str, sub: &str, value: &str) -> bool {
    let blob = open(&approved_path(hive, sub), KEY_READ)
        .ok()
        .and_then(|k| k.get_raw_value(value).ok());
    wp::approved_enabled(blob.as_ref().map(|v| v.bytes.as_slice()))
}

pub fn autostart_list() -> Result<Vec<AutostartEntry>> {
    let mut out = Vec::new();
    for (place, prefix, hive, key, sub) in RUN {
        let location = format!(r"{hive}\{key}");
        let Ok(k) = open(&location, KEY_READ) else {
            continue;
        };
        for (name, value) in k.enum_values().flatten() {
            let Ok(command) = String::from_reg_value(&value) else {
                continue;
            };
            out.push(AutostartEntry {
                id: format!("{prefix}:{name}"),
                enabled: is_enabled(hive, sub, &name),
                name,
                command,
                place,
                location: location.clone(),
                needs_admin: hive == "HKLM",
            });
        }
    }
    let folders = [
        (
            AutostartPlace::StartupUser,
            "startup:user",
            "HKCU",
            known_folder(&FOLDERID_Startup),
        ),
        (
            AutostartPlace::StartupCommon,
            "startup:common",
            "HKLM",
            known_folder(&FOLDERID_CommonStartup),
        ),
    ];
    for (place, prefix, hive, dir) in folders {
        let Some(Ok(entries)) = dir.map(std::fs::read_dir) else {
            continue;
        };
        for path in entries.flatten().map(|e| e.path()) {
            let ext = path
                .extension()
                .map(|e| e.to_string_lossy().to_ascii_lowercase());
            if !matches!(ext.as_deref(), Some("lnk" | "exe" | "bat" | "cmd")) {
                continue;
            }
            let file = path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
            let name = path
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
            let command = path.to_string_lossy().into_owned();
            out.push(AutostartEntry {
                id: format!("{prefix}:{file}"),
                enabled: is_enabled(hive, "StartupFolder", &file),
                name,
                location: command.clone(),
                command,
                place,
                needs_admin: hive == "HKLM",
            });
        }
    }
    Ok(out)
}

pub fn autostart_set(entry: &AutostartEntry, enabled: bool) -> Result<()> {
    let file_name = || {
        Path::new(&entry.location)
            .file_name()
            .map(|f| f.to_string_lossy().into_owned())
            .ok_or_else(|| Error::Failed("Not a Startup folder entry.".into()))
    };
    let (hive, sub, value) = match entry.place {
        AutostartPlace::RunUser => ("HKCU", "Run", entry.name.clone()),
        AutostartPlace::RunMachine
            if entry.location.to_ascii_lowercase().contains("wow6432node") =>
        {
            ("HKLM", "Run32", entry.name.clone())
        }
        AutostartPlace::RunMachine => ("HKLM", "Run", entry.name.clone()),
        AutostartPlace::StartupUser => ("HKCU", "StartupFolder", file_name()?),
        AutostartPlace::StartupCommon => ("HKLM", "StartupFolder", file_name()?),
        _ => return Err(Error::Unsupported),
    };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let bytes = wp::approved_bytes(enabled, wp::filetime(now.as_secs(), now.subsec_nanos()));
    let path = approved_path(hive, sub);
    let rest = path.split_once('\\').map(|(_, r)| r).unwrap_or_default();
    let (key, _) = hive_of(hive).create_subkey_with_flags(rest, KEY_SET_VALUE | KEY_WOW64_64KEY)?;
    key.set_raw_value(
        &value,
        &winreg::RegValue {
            bytes: bytes.to_vec(),
            vtype: REG_BINARY,
        },
    )?;
    Ok(())
}
