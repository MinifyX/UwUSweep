//! Registry leftovers, their quarantine, and the key list for install traces.

use std::collections::BTreeSet;
use std::path::Path;

use winreg::enums::*;
use winreg::types::FromRegValue;
use winreg::RegKey;

use super::wp::{self, ExportKey, ExportValue};
use crate::terms::{normalize, Terms};
use crate::{
    App, Confidence, Error, ItemKind, Leftover, QuarantineItem, Result, ScanLevel, Source,
};

/// Where programs keep their settings: `Vendor` or `Vendor\Product` below.
const ROOTS: [(&str, &str); 3] = [
    ("HKCU", "Software"),
    ("HKLM", "SOFTWARE"),
    ("HKLM", r"SOFTWARE\WOW6432Node"),
];

const RUN_KEYS: [(&str, &str); 3] = [
    ("HKCU", r"Software\Microsoft\Windows\CurrentVersion\Run"),
    ("HKLM", r"SOFTWARE\Microsoft\Windows\CurrentVersion\Run"),
    (
        "HKLM",
        r"SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Run",
    ),
];

/// Opens `HKCU\…` or `HKLM\…`, always in the 64-bit view; the 32-bit keys
/// are reached through their literal `WOW6432Node` path.
pub(super) fn open(path: &str, access: u32) -> std::io::Result<RegKey> {
    let (hive, rest) = wp::split_reg_path(path).ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "not a registry path")
    })?;
    hive_of(hive).open_subkey_with_flags(rest, access | KEY_WOW64_64KEY)
}

pub(super) fn hive_of(hive: &str) -> RegKey {
    RegKey::predef(if hive == "HKLM" {
        HKEY_LOCAL_MACHINE
    } else {
        HKEY_CURRENT_USER
    })
}

fn subkeys(key: &RegKey) -> Vec<String> {
    key.enum_keys().flatten().collect()
}

fn item(
    kind: ItemKind,
    path: String,
    value: Option<String>,
    hit: (Confidence, String),
) -> Leftover {
    let needs_admin = path.starts_with("HKLM");
    Leftover {
        kind,
        path,
        value,
        size: None,
        confidence: hit.0,
        reason: hit.1,
        needs_admin,
    }
}

pub fn registry_leftovers(terms: &Terms, level: ScanLevel, app: &App) -> Vec<Leftover> {
    let mut out = Vec::new();
    let app_name = normalize(&app.name);
    for (hive, root) in ROOTS {
        let base = format!(r"{hive}\{root}");
        let Ok(rk) = open(&base, KEY_READ) else {
            continue;
        };
        for vendor in subkeys(&rk) {
            if wp::is_system_key(&vendor) {
                continue;
            }
            let Ok(vk) = rk.open_subkey_with_flags(&vendor, KEY_READ | KEY_WOW64_64KEY) else {
                continue;
            };
            let children = subkeys(&vk);
            let hits: Vec<Leftover> = children
                .iter()
                .filter(|c| !wp::is_protected_key(c))
                .filter_map(|c| {
                    let hit = terms.matches(c, level)?;
                    Some(item(
                        ItemKind::RegKey,
                        format!(r"{base}\{vendor}\{c}"),
                        None,
                        hit,
                    ))
                })
                .collect();
            if hits.is_empty() && !wp::is_protected_key(&vendor) {
                if let Some(mut hit) = terms.matches(&vendor, level) {
                    // A vendor key that holds other products is only sure
                    // when it is named after the app itself.
                    if !children.is_empty() && normalize(&vendor) != app_name {
                        hit.0 = Confidence::Medium;
                    }
                    out.push(item(
                        ItemKind::RegKey,
                        format!(r"{base}\{vendor}"),
                        None,
                        hit,
                    ));
                }
            }
            out.extend(hits);
        }
    }
    if level == ScanLevel::Deep {
        if let Some(path) = uninstall_key_path(app) {
            if open(&path, KEY_READ).is_ok() {
                out.push(item(
                    ItemKind::RegKey,
                    path,
                    None,
                    (Confidence::High, app.name.clone()),
                ));
            }
        }
        for (hive, key) in RUN_KEYS {
            let path = format!(r"{hive}\{key}");
            let Ok(rk) = open(&path, KEY_READ) else {
                continue;
            };
            for (name, value) in rk.enum_values().flatten() {
                let target = String::from_reg_value(&value).ok().map(|cmd| {
                    let file = wp::split_command_line(&cmd).0;
                    let file = file.rsplit('\\').next().unwrap_or_default().to_string();
                    file.rsplit_once('.')
                        .map_or(file.clone(), |(stem, _)| stem.to_string())
                });
                let hit = terms
                    .matches(&name, level)
                    .or_else(|| target.and_then(|t| terms.matches(&t, level)));
                if let Some(hit) = hit {
                    out.push(item(ItemKind::RegValue, path.clone(), Some(name), hit));
                }
            }
        }
        let services = r"HKLM\SYSTEM\CurrentControlSet\Services";
        if let Ok(sk) = open(services, KEY_READ) {
            for name in subkeys(&sk) {
                if wp::is_protected_key(&name) {
                    continue;
                }
                if let Some((_, term)) = terms.matches(&name, level) {
                    let path = format!(r"{services}\{name}");
                    out.push(item(
                        ItemKind::RegKey,
                        path,
                        None,
                        (Confidence::Medium, term),
                    ));
                }
            }
        }
    }
    out
}

/// The app's own `…\Uninstall\<key>` as a leftover path.
fn uninstall_key_path(app: &App) -> Option<String> {
    let Source::Registry {
        hive, key, wow64, ..
    } = &app.source
    else {
        return None;
    };
    let name = key.rsplit('\\').next().filter(|n| !n.is_empty())?;
    Some(match (hive.as_str(), wow64) {
        ("HKLM", true) => format!(r"HKLM\SOFTWARE\WOW6432Node\{}\{name}", wp::UNINSTALL_KEY),
        ("HKLM", false) => format!(r"HKLM\SOFTWARE\{}\{name}", wp::UNINSTALL_KEY),
        ("HKCU", _) => format!(r"HKCU\Software\{}\{name}", wp::UNINSTALL_KEY),
        _ => return None,
    })
}

/// Values are only ever removed from the Run keys.
fn removable_value(path: &str) -> bool {
    RUN_KEYS
        .iter()
        .any(|(h, k)| path.eq_ignore_ascii_case(&format!(r"{h}\{k}")))
}

fn export_key(key: &RegKey, path: String) -> std::io::Result<ExportKey> {
    let mut values = Vec::new();
    for v in key.enum_values() {
        let (name, v) = v?;
        values.push(ExportValue {
            name,
            data: wp::to_hex(&v.bytes),
            vtype: v.vtype as u32,
        });
    }
    let mut subkeys = Vec::new();
    for name in key.enum_keys() {
        let name = name?;
        let sub = key.open_subkey_with_flags(&name, KEY_READ | KEY_WOW64_64KEY)?;
        subkeys.push(export_key(&sub, name)?);
    }
    Ok(ExportKey {
        path,
        values,
        subkeys,
    })
}

fn refuse(path: &str) -> Error {
    Error::Failed(format!("{path} is not a registry place UwUSweep removes."))
}

pub fn quarantine_registry(item: &Leftover, dest: &Path) -> Result<()> {
    let path = item.path.as_str();
    let export = match item.kind {
        ItemKind::RegKey if wp::removable_key(path) => {
            export_key(&open(path, KEY_READ)?, path.into())?
        }
        ItemKind::RegValue if removable_value(path) => {
            let name = item.value.clone().ok_or_else(|| refuse(path))?;
            let v = open(path, KEY_READ)?.get_raw_value(&name)?;
            let value = ExportValue {
                data: wp::to_hex(&v.bytes),
                vtype: v.vtype as u32,
                name,
            };
            ExportKey {
                path: path.into(),
                values: vec![value],
                subkeys: Vec::new(),
            }
        }
        _ => return Err(refuse(path)),
    };
    if let Some(dir) = dest.parent() {
        std::fs::create_dir_all(dir)?;
    }
    // Written and synced before anything is deleted, so nothing is lost.
    let file = std::fs::File::create(dest)?;
    serde_json::to_writer_pretty(&file, &export)?;
    file.sync_all()?;
    match item.kind {
        ItemKind::RegKey => {
            let (parent, name) = path.rsplit_once('\\').ok_or_else(|| refuse(path))?;
            open(parent, KEY_ALL_ACCESS)?.delete_subkey_all(name)?;
        }
        _ => open(path, KEY_SET_VALUE)?.delete_value(&export.values[0].name)?,
    }
    Ok(())
}

fn reg_type(t: u32) -> Option<RegType> {
    Some(match t {
        0 => REG_NONE,
        1 => REG_SZ,
        2 => REG_EXPAND_SZ,
        3 => REG_BINARY,
        4 => REG_DWORD,
        5 => REG_DWORD_BIG_ENDIAN,
        6 => REG_LINK,
        7 => REG_MULTI_SZ,
        8 => REG_RESOURCE_LIST,
        9 => REG_FULL_RESOURCE_DESCRIPTOR,
        10 => REG_RESOURCE_REQUIREMENTS_LIST,
        11 => REG_QWORD,
        _ => return None,
    })
}

/// Values that are in the registry again since the export.
fn clashes(k: &ExportKey, path: &str, out: &mut Vec<String>) {
    if let Ok(key) = open(path, KEY_READ) {
        for v in &k.values {
            if key.get_raw_value(&v.name).is_ok() {
                out.push(format!(
                    r"{path}\{}",
                    if v.name.is_empty() {
                        "(Default)"
                    } else {
                        &v.name
                    }
                ));
            }
        }
        for sub in &k.subkeys {
            clashes(sub, &format!(r"{path}\{}", sub.path), out);
        }
    }
}

fn write_back(k: &ExportKey, path: &str) -> Result<()> {
    let (hive, rest) = wp::split_reg_path(path).ok_or_else(|| refuse(path))?;
    let (key, _) =
        hive_of(hive).create_subkey_with_flags(rest, KEY_ALL_ACCESS | KEY_WOW64_64KEY)?;
    for v in &k.values {
        let bad = || Error::Failed(format!(r"The saved value {path}\{} is damaged.", v.name));
        let value = winreg::RegValue {
            bytes: wp::from_hex(&v.data).ok_or_else(bad)?,
            vtype: reg_type(v.vtype).ok_or_else(bad)?,
        };
        key.set_raw_value(&v.name, &value)?;
    }
    for sub in &k.subkeys {
        write_back(sub, &format!(r"{path}\{}", sub.path))?;
    }
    Ok(())
}

pub fn restore_registry(item: &QuarantineItem, src: &Path) -> Result<()> {
    let export: ExportKey = serde_json::from_slice(&std::fs::read(src)?)?;
    let path = item.original.as_str();
    let allowed = match item.kind {
        ItemKind::RegKey => wp::removable_key(path),
        ItemKind::RegValue => removable_value(path),
        _ => false,
    };
    if !allowed || export.path != path || export.subkeys.iter().any(|s| s.path.contains('\\')) {
        return Err(refuse(path));
    }
    // Checked first, so a restore never overwrites what is there now.
    let mut found = Vec::new();
    clashes(&export, path, &mut found);
    if !found.is_empty() {
        return Err(Error::Failed(format!(
            "Not restored, these are in the registry again and were left as they are: {}",
            found.join(", ")
        )));
    }
    write_back(&export, path)
}

fn walk(key: &RegKey, path: &str, depth: usize, out: &mut BTreeSet<String>) {
    for name in subkeys(key) {
        if depth == 1 && wp::is_system_key(&name) {
            continue;
        }
        let child = format!(r"{path}\{name}");
        if depth < 3 {
            if let Ok(sub) = key.open_subkey_with_flags(&name, KEY_READ | KEY_WOW64_64KEY) {
                walk(&sub, &child, depth + 1, out);
            }
        }
        out.insert(child);
    }
}

pub fn registry_snapshot() -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for (hive, root) in ROOTS {
        let base = format!(r"{hive}\{root}");
        if let Ok(key) = open(&base, KEY_READ) {
            walk(&key, &base, 1, &mut out);
        }
        let uninstall = format!(r"{base}\{}", wp::UNINSTALL_KEY);
        if let Ok(key) = open(&uninstall, KEY_READ) {
            out.extend(
                subkeys(&key)
                    .into_iter()
                    .map(|n| format!(r"{uninstall}\{n}")),
            );
        }
    }
    out
}
