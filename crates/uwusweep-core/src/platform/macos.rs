//! macOS: `.app` bundles, the `~/Library` and `/Library` folders, launchd
//! agents and daemons, and the admin prompt through `osascript`. There is no
//! registry, and bundles have no uninstaller of their own: `uninstall` does
//! nothing and the bundle is moved into the quarantine as a leftover.

use std::collections::{BTreeSet, HashMap};
use std::ffi::CString;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::process::Command as Proc;

use super::macos_parse::{self as parse, LaunchJob};
use crate::scan::Root;
use crate::terms::Terms;
use crate::{
    App, AutostartEntry, AutostartPlace, Error, Leftover, QuarantineItem, Result, ScanLevel, Source,
};

fn home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| "/var/empty".into())
}

fn uid() -> u32 {
    // SAFETY: getuid cannot fail.
    unsafe { libc::getuid() }
}

fn writable(path: &Path) -> bool {
    let Ok(c) = CString::new(path.as_os_str().as_bytes()) else {
        return false;
    };
    // SAFETY: c is a valid NUL-terminated string.
    unsafe { libc::access(c.as_ptr(), libc::W_OK) == 0 }
}

pub fn list_apps() -> Result<Vec<App>> {
    let mut bundles = Vec::new();
    for dir in [PathBuf::from("/Applications"), home().join("Applications")] {
        let Ok(rd) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in rd.flatten() {
            let p = e.path();
            if is_bundle(&p) {
                bundles.push(p);
            } else if p.is_dir() {
                // One level of folders such as /Applications/Utilities.
                let Ok(sub) = std::fs::read_dir(&p) else {
                    continue;
                };
                bundles.extend(sub.flatten().map(|e| e.path()).filter(|p| is_bundle(p)));
            }
        }
    }
    let mut apps: Vec<App> = bundles.iter().filter_map(|b| read_bundle(b)).collect();
    apps.sort_by_key(|a| a.name.to_lowercase());
    apps.dedup_by(|a, b| a.id == b.id);
    Ok(apps)
}

fn is_bundle(p: &Path) -> bool {
    p.extension().is_some_and(|e| e.eq_ignore_ascii_case("app")) && p.is_dir()
}

fn read_bundle(bundle: &Path) -> Option<App> {
    let contents = bundle.join("Contents");
    let info = plist::Value::from_file(contents.join("Info.plist"))
        .ok()?
        .into_dictionary()?;
    let has_receipt = contents.join("_MASReceipt").exists();
    let installed_on = std::fs::metadata(bundle)
        .ok()
        .and_then(|m| m.created().or_else(|_| m.modified()).ok())
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| parse::ymd(d.as_secs()));
    // Moving the bundle away needs write access to it and to its folder.
    let needs_admin = !(writable(bundle) && bundle.parent().is_some_and(writable));
    parse::app_from_info(bundle, &info, has_receipt, installed_on, needs_admin)
}

pub fn leftover_roots(level: ScanLevel) -> Vec<Root> {
    let lib = home().join("Library");
    let deep = level == ScanLevel::Deep;
    let root = |path: PathBuf, depth, needs_admin| Root {
        path,
        depth,
        needs_admin,
    };
    let mut roots: Vec<Root> = [
        "Application Support",
        "Caches",
        "Preferences",
        "Containers",
        "Group Containers",
        "Logs",
        "Saved Application State",
        "HTTPStorages",
        "WebKit",
        "Cookies",
        "LaunchAgents",
        "Application Scripts",
    ]
    .iter()
    .map(|d| {
        root(
            lib.join(d),
            if deep && *d == "Application Support" {
                2
            } else {
                1
            },
            false,
        )
    })
    .collect();
    if deep {
        roots.extend(
            [
                "Application Support",
                "Caches",
                "Preferences",
                "LaunchAgents",
                "LaunchDaemons",
                "PrivilegedHelperTools",
                "Logs",
            ]
            .iter()
            .map(|d| root(Path::new("/Library").join(d), 1, true)),
        );
        roots.push(root("/private/var/db/receipts".into(), 1, true));
    }
    roots
}

pub fn registry_leftovers(_terms: &Terms, _level: ScanLevel, _app: &App) -> Vec<Leftover> {
    Vec::new()
}

pub fn uninstall(app: &App, _quiet: bool) -> Result<()> {
    match app.source {
        // The bundle itself is the leftover the quarantine takes.
        Source::MacApp { .. } => Ok(()),
        _ => Err(Error::Unsupported),
    }
}

const LAUNCH_DIRS: [(&str, AutostartPlace); 3] = [
    ("~/Library/LaunchAgents", AutostartPlace::LaunchAgentUser),
    ("/Library/LaunchAgents", AutostartPlace::LaunchAgentSystem),
    ("/Library/LaunchDaemons", AutostartPlace::LaunchDaemon),
];

/// Agents run in the user's GUI domain, wherever their plist is; only daemons
/// live in the system domain, and only that needs admin.
fn domain(place: AutostartPlace) -> String {
    match place {
        AutostartPlace::LaunchDaemon => "system".into(),
        _ => format!("gui/{}", uid()),
    }
}

fn print_disabled(domain: &str) -> HashMap<String, bool> {
    Proc::new("launchctl")
        .args(["print-disabled", domain])
        .output()
        .map(|o| parse::parse_print_disabled(&String::from_utf8_lossy(&o.stdout)))
        .unwrap_or_default()
}

pub fn autostart_list() -> Result<Vec<AutostartEntry>> {
    let mut out = Vec::new();
    let mut overrides: HashMap<String, HashMap<String, bool>> = HashMap::new();
    for (dir, place) in LAUNCH_DIRS {
        let dir = match dir.strip_prefix("~/") {
            Some(rest) => home().join(rest),
            None => PathBuf::from(dir),
        };
        let Ok(rd) = std::fs::read_dir(&dir) else {
            continue;
        };
        let dom = domain(place);
        let disabled = overrides
            .entry(dom.clone())
            .or_insert_with(|| print_disabled(&dom));
        let mut files: Vec<PathBuf> = rd
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == "plist"))
            .collect();
        files.sort();
        for path in files {
            let Some(job) = read_job(&path) else { continue };
            let name = job.label.clone().unwrap_or_else(|| file_stem(&path));
            let off = disabled.get(&name).copied().unwrap_or(job.disabled);
            out.push(AutostartEntry {
                id: format!("{place:?}:{}", path.display()),
                name,
                command: job.command,
                place,
                location: path.display().to_string(),
                enabled: !off,
                needs_admin: place == AutostartPlace::LaunchDaemon,
            });
        }
    }
    Ok(out)
}

fn read_job(path: &Path) -> Option<LaunchJob> {
    let dict = plist::Value::from_file(path).ok()?.into_dictionary()?;
    Some(parse::launch_job(&dict))
}

fn file_stem(p: &Path) -> String {
    p.file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
}

pub fn autostart_set(entry: &AutostartEntry, enabled: bool) -> Result<()> {
    let plist = Path::new(&entry.location);
    let label = read_job(plist)
        .and_then(|j| j.label)
        .unwrap_or_else(|| entry.name.clone());
    let dom = domain(entry.place);
    let target = format!("{dom}/{label}");
    let o = Proc::new("launchctl")
        .args([if enabled { "enable" } else { "disable" }, &target])
        .output()?;
    if !o.status.success() {
        return Err(Error::Failed(
            String::from_utf8_lossy(&o.stderr).trim().to_owned(),
        ));
    }
    // Start or stop it now too; it may already be in that state, so errors
    // here mean nothing.
    let _ = if enabled {
        Proc::new("launchctl")
            .arg("bootstrap")
            .arg(&dom)
            .arg(plist)
            .output()
    } else {
        Proc::new("launchctl").args(["bootout", &target]).output()
    };
    Ok(())
}

pub fn create_restore_point(_description: &str) -> Result<()> {
    Err(Error::Unsupported)
}

pub fn quarantine_registry(_item: &Leftover, _dest: &Path) -> Result<()> {
    Err(Error::Unsupported)
}

pub fn restore_registry(_item: &QuarantineItem, _src: &Path) -> Result<()> {
    Err(Error::Unsupported)
}

pub fn registry_snapshot() -> BTreeSet<String> {
    BTreeSet::new()
}

pub fn admin_quarantine_root() -> PathBuf {
    PathBuf::from("/Library/Application Support/UwUSweep/quarantine")
}

pub fn is_elevated() -> bool {
    // SAFETY: geteuid cannot fail.
    unsafe { libc::geteuid() == 0 }
}

pub fn run_elevated(exe: &Path, args: &[String]) -> Result<i32> {
    let script = parse::admin_script(&exe.to_string_lossy(), args);
    let o = Proc::new("/usr/bin/osascript")
        .args(["-e", &script])
        .output()?;
    if o.status.success() {
        return Ok(0);
    }
    match parse::osascript_error_code(&String::from_utf8_lossy(&o.stderr)) {
        Some(-128) => Err(Error::Cancelled),
        Some(code) if code > 0 => Ok(code),
        _ => Ok(1),
    }
}
