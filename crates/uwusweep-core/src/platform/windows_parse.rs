//! The pure parts of the Windows module: turning registry fields, command
//! output and byte blobs into UwUSweep's types and back. Kept free of Windows
//! APIs so the tests run on every system.

use serde::{Deserialize, Serialize};

use crate::{App, Command, Source};

pub const UNINSTALL_KEY: &str = r"Microsoft\Windows\CurrentVersion\Uninstall";

/// Lower case letters and digits only; enough to compare display names.
fn simple_norm(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

/// `{XXXXXXXX-XXXX-XXXX-XXXX-XXXXXXXXXXXX}`.
pub fn is_guid(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 38
        && b[0] == b'{'
        && b[37] == b'}'
        && b[1..37].iter().enumerate().all(|(i, c)| match i {
            8 | 13 | 18 | 23 => *c == b'-',
            _ => c.is_ascii_hexdigit(),
        })
}

/// `YYYYMMDD` → `YYYY-MM-DD`.
pub fn install_date(s: &str) -> Option<String> {
    let s = s.trim();
    (s.len() == 8 && s.bytes().all(|b| b.is_ascii_digit()))
        .then(|| format!("{}-{}-{}", &s[..4], &s[4..6], &s[6..]))
}

/// Expands `%NAME%` with `lookup`; unknown names stay as they are.
pub fn expand_env(s: &str, lookup: impl Fn(&str) -> Option<String>) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(start) = rest.find('%') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        match after.find('%') {
            Some(end) if end > 0 => {
                let name = &after[..end];
                match lookup(name) {
                    Some(v) => out.push_str(&v),
                    None => {
                        out.push('%');
                        out.push_str(name);
                        out.push('%');
                    }
                }
                rest = &after[end + 1..];
            }
            _ => {
                out.push('%');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// Splits a command line into program and parameters: a leading quoted path,
/// else everything up to the first `.exe` that ends a word (so unquoted paths
/// with spaces work), else up to the first space.
pub fn split_command_line(line: &str) -> (String, String) {
    let line = line.trim();
    if let Some(rest) = line.strip_prefix('"') {
        if let Some(end) = rest.find('"') {
            return (rest[..end].to_string(), rest[end + 1..].trim().to_string());
        }
        return (rest.to_string(), String::new());
    }
    let lower = line.to_ascii_lowercase();
    let mut from = 0;
    while let Some(i) = lower[from..].find(".exe") {
        let end = from + i + 4;
        if end == line.len() || line[end..].starts_with(' ') {
            return (line[..end].to_string(), line[end..].trim().to_string());
        }
        from = end;
    }
    match line.split_once(' ') {
        Some((f, p)) => (f.to_string(), p.trim().to_string()),
        None => (line.to_string(), String::new()),
    }
}

/// The file in a `DisplayIcon` value: `"C:\x\app.exe",0` → `C:\x\app.exe`.
pub fn icon_path(s: &str) -> Option<String> {
    let s = s.trim();
    let s = match s.rsplit_once(',') {
        Some((p, idx))
            if idx
                .trim()
                .trim_start_matches('-')
                .bytes()
                .all(|b| b.is_ascii_digit()) =>
        {
            p
        }
        _ => s,
    };
    let s = s.trim().trim_matches('"').trim();
    (!s.is_empty()).then(|| s.to_string())
}

/// The folder part of a Windows path and the file's stem.
fn parent_and_stem(path: &str) -> (Option<&str>, &str) {
    let (dir, file) = match path.rfind('\\') {
        Some(i) => (Some(&path[..i]), &path[i + 1..]),
        None => (None, path),
    };
    let stem = file.rsplit_once('.').map_or(file, |(s, _)| s);
    (dir, stem)
}

/// A folder an uninstaller or icon sits in is only a usable install folder
/// when it is not a system or shared place.
fn usable_install_dir(dir: &str) -> bool {
    let d = dir.to_ascii_lowercase().replace('/', "\\");
    let d = d.trim_end_matches('\\');
    let system = d.len() >= 10
        && d.as_bytes()[1] == b':'
        && (&d[2..] == "\\windows" || d[2..].starts_with("\\windows\\"));
    !(d.len() < 4
        || system
        || d.contains("\\package cache")
        || d.contains("\\common files")
        || d.contains("\\installshield installation information"))
}

/// One `…\Uninstall\<key>` entry as read from the registry, values expanded.
#[derive(Debug, Clone, Default)]
pub struct ArpEntry {
    pub hive: String,
    pub key_name: String,
    pub wow64: bool,
    pub display_name: Option<String>,
    pub display_version: Option<String>,
    pub publisher: Option<String>,
    pub install_location: Option<String>,
    pub estimated_size_kb: Option<u32>,
    pub install_date: Option<String>,
    pub uninstall_string: Option<String>,
    pub quiet_uninstall_string: Option<String>,
    pub display_icon: Option<String>,
    pub windows_installer: bool,
    pub system_component: bool,
    pub parent_key_name: bool,
    pub release_type: Option<String>,
}

fn non_empty(s: &Option<String>) -> Option<String> {
    s.as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// An uninstall entry as UwUSweep shows it, or `None` for updates, system
/// components and entries without a name.
pub fn arp_to_app(e: &ArpEntry) -> Option<App> {
    let name = non_empty(&e.display_name)?;
    if e.system_component || e.parent_key_name {
        return None;
    }
    if let Some(rt) = &e.release_type {
        let rt = rt.trim().to_ascii_lowercase();
        if matches!(rt.as_str(), "update" | "hotfix" | "security update") {
            return None;
        }
    }
    let uninstall_line = non_empty(&e.uninstall_string);
    let guid = is_guid(&e.key_name);
    let msi = guid
        && (e.windows_installer
            || uninstall_line
                .as_deref()
                .is_none_or(|l| l.to_ascii_lowercase().contains("msiexec")));
    let msi_product_code = msi.then(|| e.key_name.clone());
    let (uninstall, quiet_uninstall) = match &msi_product_code {
        Some(code) => (
            Some(format!("MsiExec.exe /X{code}")),
            Some(format!("MsiExec.exe /X{code} /qn /norestart")),
        ),
        None => (uninstall_line.clone(), non_empty(&e.quiet_uninstall_string)),
    };

    let install_dir = non_empty(&e.install_location)
        .map(|s| s.trim_matches('"').trim_end_matches('\\').to_string())
        .filter(|s| !s.is_empty())
        .or_else(|| {
            let from_icon = e.display_icon.as_deref().and_then(icon_path);
            let from_uninst = uninstall_line.as_deref().map(|l| split_command_line(l).0);
            [from_icon, from_uninst]
                .into_iter()
                .flatten()
                .find_map(|p| {
                    let (dir, stem) = parent_and_stem(&p);
                    let dir = dir?;
                    (!stem.eq_ignore_ascii_case("msiexec") && usable_install_dir(dir))
                        .then(|| dir.to_string())
                })
        });

    let mut hints = Vec::new();
    if !guid {
        let k = e.key_name.strip_suffix("_is1").unwrap_or(&e.key_name);
        hints.push(k.to_string());
    }
    if let Some(p) = e.display_icon.as_deref().and_then(icon_path) {
        let (dir, stem) = parent_and_stem(&p);
        let stem = if dir.is_none_or(usable_install_dir) {
            stem.to_string()
        } else {
            String::new()
        };
        let l = stem.to_ascii_lowercase();
        let generic = l.starts_with("unins")
            || matches!(
                l.as_str(),
                "uninstall" | "setup" | "msiexec" | "installer" | ""
            );
        if !generic && !hints.iter().any(|h: &String| h.eq_ignore_ascii_case(&stem)) {
            hints.push(stem);
        }
    }

    let root = if e.hive == "HKLM" {
        "SOFTWARE"
    } else {
        "Software"
    };
    Some(App {
        id: format!(
            "reg:{}:{}:{}",
            e.hive,
            if e.wow64 { 32 } else { 64 },
            e.key_name
        ),
        name,
        publisher: non_empty(&e.publisher),
        version: non_empty(&e.display_version),
        install_dir: install_dir.map(Into::into),
        size: e
            .estimated_size_kb
            .filter(|s| *s > 0)
            .map(|kb| u64::from(kb) * 1024),
        installed_on: e.install_date.as_deref().and_then(install_date),
        source: Source::Registry {
            hive: e.hive.clone(),
            key: format!(r"{root}\{UNINSTALL_KEY}\{}", e.key_name),
            wow64: e.wow64,
            msi_product_code,
        },
        uninstall: uninstall.map(|line| Command::Line { line }),
        quiet_uninstall: quiet_uninstall.map(|line| Command::Line { line }),
        winget_id: None,
        needs_admin: e.hive == "HKLM",
        hints,
    })
}

/// Drops entries with the same name and version as an earlier one (the same
/// program seen through both registry views); their hints are kept.
pub fn merge_duplicates(apps: Vec<App>) -> Vec<App> {
    let mut out: Vec<App> = Vec::new();
    for app in apps {
        let key = (simple_norm(&app.name), app.version.clone());
        match out
            .iter_mut()
            .find(|a| (simple_norm(&a.name), a.version.clone()) == key)
        {
            Some(first) => {
                for h in app.hints {
                    if !first.hints.contains(&h) {
                        first.hints.push(h);
                    }
                }
                if first.install_dir.is_none() {
                    first.install_dir = app.install_dir;
                }
            }
            None => out.push(app),
        }
    }
    out
}

/// One package from `Get-AppxPackage | ConvertTo-Json`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct AppxPackage {
    pub name: String,
    pub package_full_name: String,
    pub package_family_name: String,
    pub publisher: Option<String>,
    pub version: Option<String>,
}

/// `ConvertTo-Json` prints an object for one package, an array for more and
/// nothing for none.
pub fn parse_appx_json(s: &str) -> Vec<AppxPackage> {
    let s = s.trim().trim_start_matches('\u{feff}');
    if s.is_empty() {
        return Vec::new();
    }
    serde_json::from_str::<Vec<AppxPackage>>(s)
        .or_else(|_| serde_json::from_str::<AppxPackage>(s).map(|p| vec![p]))
        .unwrap_or_default()
}

/// The `CN=` part of a certificate subject.
pub fn cn_of(subject: &str) -> Option<String> {
    let start = subject.find("CN=")? + 3;
    let rest = &subject[start..];
    let cn = match rest.strip_prefix('"') {
        Some(q) => q.split('"').next().unwrap_or(""),
        None => rest.split(',').next().unwrap_or(""),
    };
    let cn = cn.trim();
    (!cn.is_empty()).then(|| cn.to_string())
}

/// `Microsoft.WindowsCalculator` → `WindowsCalculator`: the package name
/// without its publisher prefix; the manifest's real display name is often an
/// `ms-resource:` reference and slow to resolve.
pub fn store_display_name(name: &str) -> String {
    match name.split_once('.') {
        Some((_, rest)) if !rest.is_empty() => rest.to_string(),
        _ => name.to_string(),
    }
}

/// Package full names go into a PowerShell string, so only allow their own
/// alphabet.
pub fn valid_package_full_name(s: &str) -> bool {
    !s.is_empty()
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._~-".contains(&b))
}

pub fn appx_to_app(p: &AppxPackage) -> Option<App> {
    if !valid_package_full_name(&p.package_full_name) || p.name.starts_with("ms-resource:") {
        return None;
    }
    Some(App {
        id: format!("store:{}", p.package_family_name),
        name: store_display_name(&p.name),
        publisher: p.publisher.as_deref().and_then(cn_of),
        version: p.version.clone(),
        // The WindowsApps folder is the system's; Remove-AppxPackage cleans it.
        install_dir: None,
        size: None,
        installed_on: None,
        source: Source::Store {
            package_full_name: p.package_full_name.clone(),
            package_family_name: p.package_family_name.clone(),
        },
        uninstall: Some(Command::Argv {
            argv: remove_appx_argv(&p.package_full_name),
        }),
        quiet_uninstall: None,
        winget_id: None,
        needs_admin: false,
        hints: vec![p.package_family_name.clone(), p.name.clone()],
    })
}

pub fn powershell_argv(script: &str) -> Vec<String> {
    ["powershell", "-NoProfile", "-NonInteractive", "-Command"]
        .iter()
        .map(|s| s.to_string())
        .chain([format!(
            "[Console]::OutputEncoding=[Text.Encoding]::UTF8; $ErrorActionPreference='Stop'; {script}"
        )])
        .collect()
}

/// Callers check the name with [`valid_package_full_name`] first.
pub fn remove_appx_argv(package_full_name: &str) -> Vec<String> {
    powershell_argv(&format!(
        "Remove-AppxPackage -Package '{package_full_name}'"
    ))
}

/// Restore point descriptions go into a PowerShell string.
pub fn restore_point_description(s: &str) -> String {
    let d: String = s
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || " ._:-".contains(*c))
        .take(64)
        .collect();
    if d.trim().is_empty() {
        "UwUSweep".to_string()
    } else {
        d
    }
}

/// One row of `winget list`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WingetRow {
    /// `None` when winget cut it short.
    pub name: Option<String>,
    pub id: String,
    pub version: Option<String>,
}

/// What a terminal would show for one line: the text after the last carriage
/// return, without ANSI escapes or progress bar blocks.
fn clean_line(line: &str) -> String {
    let line = line.rsplit('\r').next().unwrap_or("");
    let mut out = String::new();
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            if chars.peek() == Some(&'[') {
                chars.next();
                for c in chars.by_ref() {
                    if c.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
            continue;
        }
        if !matches!(c, '\u{2580}'..='\u{259f}') {
            out.push(c);
        }
    }
    out
}

/// Parses `winget list`. The header is localised, so columns come from where
/// the header's words start (the line above the dashes) and their order:
/// Name, Id, Version, then Available and Source when present.
pub fn parse_winget_list(out: &str) -> Vec<WingetRow> {
    let lines: Vec<String> = out.split('\n').map(clean_line).collect();
    let Some(dash) = lines.iter().position(|l| {
        let t = l.trim();
        t.len() >= 10 && t.bytes().all(|b| b == b'-')
    }) else {
        return Vec::new();
    };
    let Some(header) = lines[..dash].iter().rev().find(|l| !l.trim().is_empty()) else {
        return Vec::new();
    };
    // shortcut: counts chars, not display cells; East Asian wide names shift
    // later columns. Upgrade if such reports come in.
    let header: Vec<char> = header.chars().collect();
    let starts: Vec<usize> = (0..header.len())
        .filter(|&i| header[i] != ' ' && (i == 0 || header[i - 1] == ' '))
        .collect();
    if starts.len() < 3 {
        return Vec::new();
    }
    let mut rows = Vec::new();
    for line in &lines[dash + 1..] {
        if line.trim().is_empty() {
            break;
        }
        let chars: Vec<char> = line.chars().collect();
        let field = |i: usize| -> String {
            let from = starts[i].min(chars.len());
            let to = starts
                .get(i + 1)
                .copied()
                .unwrap_or(chars.len())
                .min(chars.len());
            chars[from..to]
                .iter()
                .collect::<String>()
                .trim()
                .to_string()
        };
        let (name, id, version) = (field(0), field(1), field(2));
        if id.is_empty() || id.contains('…') || id.contains(' ') {
            continue;
        }
        let version = version.trim_start_matches(['>', '<']).trim().to_string();
        rows.push(WingetRow {
            name: (!name.is_empty() && !name.contains('…')).then_some(name),
            id,
            version: (!version.is_empty() && !version.contains('…')).then_some(version),
        });
    }
    rows
}

/// Sets `winget_id` on the apps winget knows: `ARP\…\<key>` and `MSIX\<full
/// name>` ids by their key, the rest by name and version.
pub fn match_winget(apps: &mut [App], rows: &[WingetRow]) {
    for row in rows {
        let parts: Vec<&str> = row.id.split('\\').collect();
        let hit = match parts.as_slice() {
            ["ARP", scope, arch, key] => apps.iter_mut().find(|a| match &a.source {
                Source::Registry { hive, wow64, .. } => {
                    let hive_ok = (scope.eq_ignore_ascii_case("machine") && hive == "HKLM")
                        || (scope.eq_ignore_ascii_case("user") && hive == "HKCU");
                    let arch_ok = hive == "HKCU" || arch.eq_ignore_ascii_case("x86") == *wow64;
                    hive_ok && arch_ok && a.id.splitn(4, ':').nth(3) == Some(*key)
                }
                _ => false,
            }),
            ["MSIX", full] => apps.iter_mut().find(
                |a| matches!(&a.source, Source::Store { package_full_name, .. } if package_full_name == full),
            ),
            _ => row.name.as_ref().and_then(|name| {
                let n = simple_norm(name);
                apps.iter_mut().find(|a| {
                    a.winget_id.is_none()
                        && simple_norm(&a.name) == n
                        && (a.version.is_none() || a.version == row.version)
                })
            }),
        };
        if let Some(app) = hit {
            app.winget_id = Some(row.id.clone());
        }
    }
}

/// Quotes one argument the way `CommandLineToArgvW` and the C runtime read it.
pub fn quote_arg(arg: &str) -> String {
    if !arg.is_empty() && !arg.contains([' ', '\t', '\n', '\u{b}', '"']) {
        return arg.to_string();
    }
    let mut out = String::from("\"");
    let mut backslashes = 0;
    for c in arg.chars() {
        match c {
            '\\' => backslashes += 1,
            '"' => {
                out.extend(std::iter::repeat_n('\\', backslashes * 2 + 1));
                out.push('"');
                backslashes = 0;
            }
            _ => {
                out.extend(std::iter::repeat_n('\\', backslashes));
                out.push(c);
                backslashes = 0;
            }
        }
    }
    out.extend(std::iter::repeat_n('\\', backslashes * 2));
    out.push('"');
    out
}

pub fn join_args(args: &[String]) -> String {
    args.iter()
        .map(|a| quote_arg(a))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Key names that are never a leftover, as lower case letters and digits:
/// system keys and words too generic to belong to one program.
const PROTECTED_KEYS: &[&str] = &[
    "microsoft",
    "windows",
    "windowsnt",
    "classes",
    "policies",
    "wow6432node",
    "clients",
    "registeredapplications",
    "intel",
    "amd",
    "nvidia",
    "nvidiacorporation",
    "realtek",
    "khronos",
    "odbc",
    "partner",
    "appdatalow",
    "defaultusercontent",
    "software",
    "system",
    "currentversion",
    "currentcontrolset",
    "controlset001",
    "services",
    "uninstall",
    "run",
    "runonce",
    "explorer",
    "settings",
    "config",
    "data",
    "common",
    "shared",
    "update",
    "updater",
    "app",
    "apps",
    "program",
    "programs",
    "tools",
    "default",
    "user",
    "users",
    "setup",
    "install",
    "installer",
    "classic",
    "client",
    "drivers",
    "fonts",
    "printers",
    "description",
    "hardware",
    "sam",
    "security",
    "components",
    "google",
    "mozilla",
    "apple",
    "oracle",
    "javasoft",
    "adobe",
];

/// Keys the search never even looks into.
pub fn is_system_key(name: &str) -> bool {
    let n = simple_norm(name);
    n.starts_with("microsoft")
        || matches!(
            n.as_str(),
            "windows"
                | "classes"
                | "policies"
                | "wow6432node"
                | "clients"
                | "registeredapplications"
                | "appdatalow"
        )
}

pub fn is_protected_key(name: &str) -> bool {
    let n = simple_norm(name);
    n.len() < 3 || n.starts_with("microsoft") || PROTECTED_KEYS.contains(&n.as_str())
}

/// `HKCU\Software\X` → (`HKCU`, `Software\X`). Only the two hives UwUSweep
/// touches.
pub fn split_reg_path(path: &str) -> Option<(&str, &str)> {
    let (hive, rest) = path.split_once('\\')?;
    (matches!(hive, "HKCU" | "HKLM") && !rest.is_empty() && !rest.contains("\\\\"))
        .then_some((hive, rest))
}

/// A key may only be quarantined when it is deep enough and its own name is
/// not a system one, whatever the page sent.
pub fn removable_key(path: &str) -> bool {
    match split_reg_path(path) {
        Some((_, rest)) => {
            let parts: Vec<&str> = rest.split('\\').collect();
            parts.len() >= 2
                && parts.iter().all(|p| !p.is_empty())
                && !is_protected_key(parts[parts.len() - 1])
        }
        None => false,
    }
}

/// A registry key as it is stored in the quarantine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportKey {
    /// The full path for the exported key, the bare name for subkeys.
    pub path: String,
    pub values: Vec<ExportValue>,
    pub subkeys: Vec<ExportKey>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportValue {
    pub name: String,
    /// The `REG_*` type number.
    pub vtype: u32,
    /// The raw bytes, hex.
    pub data: String,
}

pub fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn from_hex(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok())
        .collect()
}

/// `StartupApproved` blobs: an even first byte means on; a missing value
/// too.
pub fn approved_enabled(bytes: Option<&[u8]>) -> bool {
    bytes.and_then(|b| b.first()).is_none_or(|b| b % 2 == 0)
}

/// The 12 bytes Task Manager writes: a flag, then when it was turned off.
pub fn approved_bytes(enabled: bool, filetime: u64) -> [u8; 12] {
    let mut b = [0u8; 12];
    b[0] = if enabled { 2 } else { 3 };
    if !enabled {
        b[4..].copy_from_slice(&filetime.to_le_bytes());
    }
    b
}

/// 100 ns intervals since 1601, from seconds and nanoseconds since 1970.
pub fn filetime(unix_secs: u64, nanos: u32) -> u64 {
    unix_secs * 10_000_000 + u64::from(nanos) / 100 + 116_444_736_000_000_000
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(key: &str, name: &str) -> ArpEntry {
        ArpEntry {
            hive: "HKLM".into(),
            key_name: key.into(),
            display_name: Some(name.into()),
            ..Default::default()
        }
    }

    #[test]
    fn guid_and_date() {
        assert!(is_guid("{12345678-ABCD-ef01-2345-6789ABCDEF01}"));
        assert!(!is_guid("{12345678-ABCD-ef01-2345-6789ABCDEF01}_is1"));
        assert!(!is_guid("12345678-ABCD-ef01-2345-6789ABCDEF01"));
        assert_eq!(install_date("20240131").as_deref(), Some("2024-01-31"));
        assert_eq!(install_date("31.01.2024"), None);
    }

    #[test]
    fn env_expansion() {
        let look = |n: &str| (n == "ProgramFiles").then(|| r"C:\Program Files".to_string());
        assert_eq!(
            expand_env(r"%ProgramFiles%\X\u.exe", look),
            r"C:\Program Files\X\u.exe"
        );
        assert_eq!(expand_env("50% %Nope% x%", look), "50% %Nope% x%");
    }

    #[test]
    fn command_lines_split() {
        assert_eq!(
            split_command_line(r#""C:\Program Files\X\unins000.exe" /SILENT"#),
            (r"C:\Program Files\X\unins000.exe".into(), "/SILENT".into())
        );
        assert_eq!(
            split_command_line(r"C:\Program Files\X Y\uninstall.exe /S"),
            (r"C:\Program Files\X Y\uninstall.exe".into(), "/S".into())
        );
        assert_eq!(
            split_command_line(r"C:\a.exed\u.EXE"),
            (r"C:\a.exed\u.EXE".into(), String::new())
        );
        assert_eq!(
            split_command_line("MsiExec.exe /X{A}"),
            ("MsiExec.exe".into(), "/X{A}".into())
        );
        assert_eq!(
            icon_path(r#""C:\X\app.exe",0"#).as_deref(),
            Some(r"C:\X\app.exe")
        );
        assert_eq!(icon_path(r"C:\X\app.ico").as_deref(), Some(r"C:\X\app.ico"));
    }

    #[test]
    fn msi_entry() {
        let code = "{12345678-ABCD-EF01-2345-6789ABCDEF01}";
        let mut e = entry(code, "Tool");
        e.windows_installer = true;
        e.uninstall_string = Some(format!("MsiExec.exe /I{code}"));
        e.estimated_size_kb = Some(2);
        e.install_date = Some("20230102".into());
        e.wow64 = true;
        let app = arp_to_app(&e).unwrap();
        assert_eq!(app.id, format!("reg:HKLM:32:{code}"));
        assert_eq!(
            app.uninstall,
            Some(Command::Line {
                line: format!("MsiExec.exe /X{code}")
            })
        );
        assert_eq!(
            app.quiet_uninstall,
            Some(Command::Line {
                line: format!("MsiExec.exe /X{code} /qn /norestart")
            })
        );
        assert_eq!(app.size, Some(2048));
        assert_eq!(app.installed_on.as_deref(), Some("2023-01-02"));
        assert!(app.needs_admin);
        assert!(app.hints.is_empty());
        assert!(app.install_dir.is_none(), "msiexec is no install folder");
        match app.source {
            Source::Registry {
                key,
                msi_product_code,
                ..
            } => {
                assert_eq!(key, format!(r"SOFTWARE\{UNINSTALL_KEY}\{code}"));
                assert_eq!(msi_product_code.as_deref(), Some(code));
            }
            _ => panic!(),
        }
    }

    #[test]
    fn guid_key_with_own_uninstaller_is_not_msi() {
        let mut e = entry("{12345678-ABCD-EF01-2345-6789ABCDEF01}", "Nsis Thing");
        e.uninstall_string = Some(r#""C:\Program Files\Thing\uninst.exe""#.into());
        let app = arp_to_app(&e).unwrap();
        assert!(matches!(
            app.source,
            Source::Registry {
                msi_product_code: None,
                ..
            }
        ));
        assert_eq!(
            app.install_dir.as_deref(),
            Some(std::path::Path::new(r"C:\Program Files\Thing"))
        );
    }

    #[test]
    fn skipped_entries() {
        assert!(arp_to_app(&ArpEntry {
            key_name: "x".into(),
            ..Default::default()
        })
        .is_none());
        let mut e = entry("KB1", "Update for X");
        e.parent_key_name = true;
        assert!(arp_to_app(&e).is_none());
        let mut e = entry("K", "X");
        e.system_component = true;
        assert!(arp_to_app(&e).is_none());
        let mut e = entry("K", "X");
        e.release_type = Some("Security Update".into());
        assert!(arp_to_app(&e).is_none());
    }

    #[test]
    fn install_dir_fallback_and_hints() {
        let mut e = entry("Thing_is1", "Thing");
        e.hive = "HKCU".into();
        e.display_icon = Some(r#""C:\Users\u\AppData\Local\Thing\thing.exe",0"#.into());
        e.uninstall_string = Some(r#""C:\Users\u\AppData\Local\Thing\unins000.exe""#.into());
        let app = arp_to_app(&e).unwrap();
        assert_eq!(
            app.install_dir.as_deref(),
            Some(std::path::Path::new(r"C:\Users\u\AppData\Local\Thing"))
        );
        assert_eq!(app.hints, vec!["Thing".to_string()]);
        assert!(!app.needs_admin);

        let mut e = entry("Other", "Other");
        e.display_icon = Some(r"C:\WINDOWS\system32\shell32.dll,-12".into());
        e.uninstall_string = Some(r"C:\Windows\uninstaller.exe /x".into());
        let app = arp_to_app(&e).unwrap();
        assert!(app.install_dir.is_none());
        assert_eq!(app.hints, vec!["Other".to_string()]);
    }

    #[test]
    fn duplicates_merge() {
        let mut a = entry("A", "Tool");
        a.display_version = Some("1.0".into());
        let mut b = a.clone();
        b.wow64 = true;
        b.key_name = "B".into();
        let mut c = a.clone();
        c.display_version = Some("2.0".into());
        let apps = merge_duplicates([&a, &b, &c].iter().filter_map(|e| arp_to_app(e)).collect());
        assert_eq!(apps.len(), 2);
        assert_eq!(apps[0].hints, vec!["A".to_string(), "B".to_string()]);
    }

    #[test]
    fn appx() {
        let one = r#"{"Name":"Contoso.Notes","PackageFullName":"Contoso.Notes_1.2.3.0_x64__abc123","PackageFamilyName":"Contoso.Notes_abc123","Publisher":"CN=\"Contoso, Ltd.\", O=Contoso, C=US","Version":"1.2.3.0","InstallLocation":"C:\\Program Files\\WindowsApps\\x"}"#;
        let list = parse_appx_json(one);
        assert_eq!(list.len(), 1);
        let app = appx_to_app(&list[0]).unwrap();
        assert_eq!(app.id, "store:Contoso.Notes_abc123");
        assert_eq!(app.name, "Notes");
        assert_eq!(app.publisher.as_deref(), Some("Contoso, Ltd."));
        assert_eq!(parse_appx_json(&format!("[{one},{one}]")).len(), 2);
        assert!(parse_appx_json("").is_empty());
        assert_eq!(
            cn_of("CN=Fabrikam, O=Fabrikam").as_deref(),
            Some("Fabrikam")
        );
        assert!(!valid_package_full_name("x'; Remove-Item C:\\ #"));
        let bad = AppxPackage {
            package_full_name: "a'b".into(),
            ..list[0].clone()
        };
        assert!(appx_to_app(&bad).is_none());
        assert_eq!(restore_point_description("Before 'x'; rm"), "Before x rm");
    }

    const WINGET_EN: &str = "   - \r   \\ \r   | \r                                                                                                                        \rName                     Id                          Version      Available Source\n-------------------------------------------------------------------------------------------\nContoso Notes            Contoso.Notes               1.2.3        1.3.0     winget\nThing                    ARP\\Machine\\X64\\Thing_is1   4.0\nVery Long Application N… Fabrikam.VeryLongApplicat…  2.0                    winget\nTruncated Name Of App …  Fabrikam.Tool               > 5.1                  winget\n\n2 upgrades available.\n";

    #[test]
    fn winget_tables() {
        let rows = parse_winget_list(WINGET_EN);
        assert_eq!(
            rows,
            vec![
                WingetRow {
                    name: Some("Contoso Notes".into()),
                    id: "Contoso.Notes".into(),
                    version: Some("1.2.3".into())
                },
                WingetRow {
                    name: Some("Thing".into()),
                    id: r"ARP\Machine\X64\Thing_is1".into(),
                    version: Some("4.0".into())
                },
                WingetRow {
                    name: None,
                    id: "Fabrikam.Tool".into(),
                    version: Some("5.1".into())
                },
            ]
        );
        let row = |a: &str, b: &str, c: &str, d: &str| format!("{a:<15}{b:<37}{c:<9}{d}\n");
        let de = format!(
            "\u{1b}[2K\u{2588}\u{2588}\u{2592}\r\n{}{}\n{}{}",
            row("Name", "ID", "Version", "Quelle"),
            "-".repeat(66),
            row("Kontakte Plus", "Contoso.Kontakte", "2.0.1", "winget"),
            row(
                "Paket",
                r"MSIX\Contoso.Paket_1.0.0.0_x64__abc",
                "1.0.0.0",
                ""
            ),
        );
        let rows = parse_winget_list(&de);
        assert_eq!(rows.len(), 2, "{rows:?}");
        assert_eq!(rows[0].id, "Contoso.Kontakte");
        assert_eq!(rows[1].id, r"MSIX\Contoso.Paket_1.0.0.0_x64__abc");
        assert_eq!(rows[1].version.as_deref(), Some("1.0.0.0"));
        assert!(parse_winget_list("Es wurde kein installiertes Paket gefunden.").is_empty());
    }

    #[test]
    fn winget_matching() {
        let mut thing = entry("Thing_is1", "Thing");
        thing.display_version = Some("4.0".into());
        let mut notes = entry("{12345678-ABCD-EF01-2345-6789ABCDEF01}", "Contoso Notes");
        notes.hive = "HKCU".into();
        notes.display_version = Some("1.2.3".into());
        let mut apps: Vec<App> = [thing, notes].iter().filter_map(arp_to_app).collect();
        match_winget(&mut apps, &parse_winget_list(WINGET_EN));
        assert_eq!(
            apps[0].winget_id.as_deref(),
            Some(r"ARP\Machine\X64\Thing_is1")
        );
        assert_eq!(apps[1].winget_id.as_deref(), Some("Contoso.Notes"));
    }

    #[test]
    fn quoting() {
        assert_eq!(quote_arg("plain"), "plain");
        assert_eq!(quote_arg(""), "\"\"");
        assert_eq!(quote_arg(r"C:\Program Files\x"), r#""C:\Program Files\x""#);
        assert_eq!(quote_arg(r"C:\dir with space\"), r#""C:\dir with space\\""#);
        assert_eq!(quote_arg(r#"a"b"#), r#""a\"b""#);
        assert_eq!(quote_arg(r#"a\"b"#), r#""a\\\"b""#);
        assert_eq!(join_args(&["--x".into(), "a b".into()]), r#"--x "a b""#);
    }

    #[test]
    fn protected_and_paths() {
        assert!(is_protected_key("Microsoft"));
        assert!(is_protected_key("Microsoft Corporation"));
        assert!(is_protected_key("WOW6432Node"));
        assert!(is_protected_key("Settings"));
        assert!(!is_protected_key("Contoso"));
        assert!(is_system_key("Classes") && !is_system_key("Google"));
        assert_eq!(
            split_reg_path(r"HKLM\SOFTWARE\X"),
            Some(("HKLM", r"SOFTWARE\X"))
        );
        assert_eq!(split_reg_path(r"HKCR\X"), None);
        assert!(removable_key(r"HKCU\Software\Contoso"));
        assert!(removable_key(
            r"HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\Thing"
        ));
        assert!(!removable_key(r"HKCU\Software"));
        assert!(!removable_key(r"HKLM\SOFTWARE\Microsoft"));
        assert!(!removable_key(r"HKLM\SOFTWARE\WOW6432Node"));
        assert!(!removable_key(r"HKCU\Software\\Contoso"));
    }

    #[test]
    fn export_format_round_trip() {
        let k = ExportKey {
            path: r"HKCU\Software\Contoso".into(),
            values: vec![ExportValue {
                name: "".into(),
                vtype: 1,
                data: to_hex(&[b'a', 0, 0, 0]),
            }],
            subkeys: vec![ExportKey {
                path: "Sub".into(),
                values: vec![],
                subkeys: vec![],
            }],
        };
        let json = serde_json::to_string(&k).unwrap();
        assert_eq!(serde_json::from_str::<ExportKey>(&json).unwrap(), k);
        assert_eq!(from_hex("00ff10").unwrap(), vec![0, 255, 16]);
        assert_eq!(from_hex("0"), None);
        assert_eq!(from_hex("zz"), None);
    }

    #[test]
    fn startup_approved() {
        assert!(approved_enabled(None));
        assert!(approved_enabled(Some(&[2, 0, 0, 0])));
        assert!(approved_enabled(Some(&[6])));
        assert!(!approved_enabled(Some(&[3, 0, 0, 0])));
        assert_eq!(
            approved_bytes(true, 99),
            [2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]
        );
        let off = approved_bytes(false, 0x0102030405060708);
        assert_eq!(off, [3, 0, 0, 0, 8, 7, 6, 5, 4, 3, 2, 1]);
        assert!(!approved_enabled(Some(&off)));
        assert_eq!(filetime(0, 0), 116_444_736_000_000_000);
    }
}
