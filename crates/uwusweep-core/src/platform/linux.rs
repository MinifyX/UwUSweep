//! Linux: system packages that own a `.desktop` file (dpkg, pacman, rpm),
//! Flatpak and Snap; XDG autostart and `systemctl --user` units.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::process::Command as Process;

use walkdir::WalkDir;

use crate::autostart::{exec_display, exec_program, parse_desktop, write_override, DesktopEntry};
use crate::scan::{home_dir, Root};
use crate::terms::Terms;
use crate::{
    App, AutostartEntry, AutostartPlace, Command, Error, Leftover, PackageManager, QuarantineItem,
    Result, ScanLevel, Source,
};

fn home() -> PathBuf {
    home_dir().unwrap_or_else(|| PathBuf::from("/nonexistent"))
}

fn in_path(program: &str) -> bool {
    std::env::var_os("PATH")
        .is_some_and(|p| std::env::split_paths(&p).any(|d| d.join(program).is_file()))
}

/// Runs a query and returns stdout, whatever the exit code (dpkg -S fails
/// when one of many paths is not owned but still prints the others).
fn output(program: &str, args: &[&str]) -> Option<String> {
    let out = Process::new(program)
        .args(args)
        .env("LC_ALL", "C")
        .output()
        .ok()?;
    Some(String::from_utf8_lossy(&out.stdout).into_owned())
}

// ---------------------------------------------------------------- app list

struct Desktop {
    path: PathBuf,
    entry: DesktopEntry,
}

fn desktop_files(dir: &Path) -> Vec<Desktop> {
    let mut out: Vec<Desktop> = WalkDir::new(dir)
        .max_depth(2)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|x| x == "desktop"))
        .filter_map(|e| {
            let entry = parse_desktop(&fs::read_to_string(e.path()).ok()?);
            let shown =
                entry.kind.as_deref() == Some("Application") && !entry.no_display && !entry.hidden;
            shown.then(|| Desktop {
                path: e.path().to_path_buf(),
                entry,
            })
        })
        .collect();
    out.sort_by(|a, b| a.path.cmp(&b.path));
    out
}

fn stem(path: &Path) -> String {
    path.file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn hints_for(desktops: &[&Desktop], extra: &str) -> Vec<String> {
    let mut hints = vec![extra.to_string()];
    for d in desktops {
        hints.extend(d.entry.exec.as_deref().and_then(exec_program));
        hints.push(stem(&d.path));
    }
    hints.sort();
    hints.dedup();
    hints
}

fn base_app(id: String, name: String, source: Source) -> App {
    App {
        id,
        name,
        publisher: None,
        version: None,
        install_dir: None,
        size: None,
        installed_on: None,
        source,
        uninstall: None,
        quiet_uninstall: None,
        winget_id: None,
        needs_admin: true,
        hints: Vec::new(),
    }
}

pub fn list_apps() -> Result<Vec<App>> {
    let home = home();
    let mut system = Vec::new();
    for dir in ["/usr/share/applications", "/usr/local/share/applications"] {
        system.extend(desktop_files(Path::new(dir)));
    }
    // ~/.local/share/applications holds the user's own launchers (AppImages
    // and the like): no package to remove, so not listed in v1.
    let mut apps = package_apps(&system);
    apps.extend(flatpak_apps(&home));
    apps.extend(snap_apps());
    apps.sort_by_key(|a| a.name.to_lowercase());
    Ok(apps)
}

fn detect_manager() -> Option<PackageManager> {
    if Path::new("/var/lib/dpkg/status").exists() && in_path("dpkg-query") {
        Some(PackageManager::Dpkg)
    } else if Path::new("/var/lib/pacman/local").exists() && in_path("pacman") {
        Some(PackageManager::Pacman)
    } else if in_path("rpm") {
        Some(PackageManager::Rpm)
    } else {
        None
    }
}

struct PkgInfo {
    version: Option<String>,
    size: Option<u64>,
}

fn package_apps(desktops: &[Desktop]) -> Vec<App> {
    let Some(manager) = detect_manager() else {
        return Vec::new();
    };
    let paths: Vec<String> = desktops
        .iter()
        .map(|d| d.path.to_string_lossy().into_owned())
        .collect();
    let mut owner: HashMap<String, String> = HashMap::new();
    let mut info: HashMap<String, PkgInfo> = HashMap::new();
    for chunk in paths.chunks(200) {
        let args: Vec<&str> = chunk.iter().map(String::as_str).collect();
        match manager {
            PackageManager::Dpkg => {
                let mut a = vec!["-S"];
                a.extend(&args);
                owner.extend(
                    output("dpkg", &a)
                        .map(|o| parse_dpkg_s(&o))
                        .unwrap_or_default(),
                );
            }
            PackageManager::Pacman => {
                let mut a = vec!["-Qo"];
                a.extend(&args);
                for (path, pkg, version) in output("pacman", &a)
                    .map(|o| parse_pacman_qo(&o))
                    .unwrap_or_default()
                {
                    info.entry(pkg.clone()).or_insert(PkgInfo {
                        version: Some(version),
                        size: None,
                    });
                    owner.insert(path, pkg);
                }
            }
            PackageManager::Rpm => {
                for (path, (name, version, size)) in rpm_owners(chunk) {
                    info.entry(name.clone()).or_insert(PkgInfo {
                        version: Some(version),
                        size,
                    });
                    owner.insert(path, name);
                }
            }
        }
    }
    let mut groups: BTreeMap<String, Vec<&Desktop>> = BTreeMap::new();
    for (d, path) in desktops.iter().zip(&paths) {
        if let Some(pkg) = owner.get(path).filter(|p| valid_name(p)) {
            groups.entry(pkg.clone()).or_default().push(d);
        }
    }
    let pkgs: Vec<&str> = groups.keys().map(String::as_str).collect();
    match manager {
        PackageManager::Dpkg if !pkgs.is_empty() => {
            let mut a = vec!["-W", "-f=${Package}\t${Version}\t${Installed-Size}\n"];
            a.extend(&pkgs);
            for (pkg, (version, size)) in output("dpkg-query", &a)
                .map(|o| parse_dpkg_query(&o))
                .unwrap_or_default()
            {
                info.insert(
                    pkg,
                    PkgInfo {
                        version: Some(version),
                        size,
                    },
                );
            }
        }
        PackageManager::Pacman if !pkgs.is_empty() => {
            let mut a = vec!["-Qi"];
            a.extend(&pkgs);
            for (pkg, (version, size)) in output("pacman", &a)
                .map(|o| parse_pacman_qi(&o))
                .unwrap_or_default()
            {
                info.insert(
                    pkg,
                    PkgInfo {
                        version: Some(version),
                        size,
                    },
                );
            }
        }
        _ => {}
    }
    groups
        .into_iter()
        .map(|(pkg, ds)| {
            let name = ds[0].entry.name.clone().unwrap_or_else(|| pkg.clone());
            let tag = match manager {
                PackageManager::Dpkg => "dpkg",
                PackageManager::Pacman => "pacman",
                PackageManager::Rpm => "rpm",
            };
            let source = Source::Package {
                manager,
                name: pkg.clone(),
            };
            let mut app = base_app(format!("{tag}:{pkg}"), name, source);
            if let Some(i) = info.remove(&pkg) {
                app.version = i.version;
                app.size = i.size;
            }
            app.hints = hints_for(&ds, &pkg);
            app.uninstall = uninstall_argv(&app).ok().map(|argv| Command::Argv { argv });
            app
        })
        .collect()
}

/// `pkg1, pkg2: /path` → path → first package. Diversion lines are skipped.
pub fn parse_dpkg_s(out: &str) -> HashMap<String, String> {
    out.lines()
        .filter(|l| !l.starts_with("diversion"))
        .filter_map(|l| {
            let (pkgs, path) = l.split_once(": ")?;
            let pkg = pkgs.split(',').next()?.trim();
            // "pkg:amd64" is the multiarch form; apt removes it by that name too.
            path.starts_with('/')
                .then(|| (path.trim().to_string(), pkg.to_string()))
        })
        .collect()
}

/// `${Package}\t${Version}\t${Installed-Size}` (KiB).
pub fn parse_dpkg_query(out: &str) -> HashMap<String, (String, Option<u64>)> {
    out.lines()
        .filter_map(|l| {
            let mut f = l.split('\t');
            let (pkg, version) = (f.next()?, f.next()?);
            let size = f
                .next()
                .and_then(|s| s.trim().parse::<u64>().ok())
                .map(|k| k.saturating_mul(1024));
            (!pkg.is_empty()).then(|| (pkg.to_string(), (version.to_string(), size)))
        })
        .collect()
}

/// `/usr/share/applications/x.desktop is owned by x 1.2-1`.
pub fn parse_pacman_qo(out: &str) -> Vec<(String, String, String)> {
    out.lines()
        .filter_map(|l| {
            let (path, rest) = l.split_once(" is owned by ")?;
            let (pkg, version) = rest.trim().split_once(' ')?;
            Some((path.to_string(), pkg.to_string(), version.to_string()))
        })
        .collect()
}

/// `pacman -Qi` blocks: `Name`, `Version`, `Installed Size : 12.50 MiB`.
pub fn parse_pacman_qi(out: &str) -> HashMap<String, (String, Option<u64>)> {
    let mut map = HashMap::new();
    for block in out.split("\n\n") {
        let mut name = None;
        let mut version = None;
        let mut size = None;
        for line in block.lines() {
            let Some((k, v)) = line.split_once(':') else {
                continue;
            };
            match k.trim() {
                "Name" => name = Some(v.trim().to_string()),
                "Version" => version = Some(v.trim().to_string()),
                "Installed Size" => size = parse_size(v),
                _ => {}
            }
        }
        if let (Some(n), Some(v)) = (name, version) {
            map.insert(n, (v, size));
        }
    }
    map
}

/// `12.50 MiB`, `1.2 GB`, `512 B`, `3,4 kB` → bytes.
pub fn parse_size(s: &str) -> Option<u64> {
    let s = s.trim().replace(',', ".");
    let (num, unit) = s.split_once(char::is_whitespace).unwrap_or((&s, "B"));
    let n: f64 = num.parse().ok()?;
    let mult = match unit.trim().to_ascii_lowercase().as_str() {
        "b" | "bytes" => 1.0,
        "kb" => 1e3,
        "kib" | "k" => 1024.0,
        "mb" => 1e6,
        "mib" | "m" => 1024.0 * 1024.0,
        "gb" => 1e9,
        "gib" | "g" => 1024.0 * 1024.0 * 1024.0,
        "tb" => 1e12,
        "tib" => 1024f64.powi(4),
        _ => return None,
    };
    Some((n * mult) as u64)
}

/// One line of `rpm -qf --qf '%{NAME}\t%{VERSION}-%{RELEASE}\t%{SIZE}\n'`;
/// `None` for "file … is not owned by any package".
pub fn parse_rpm_line(line: &str) -> Option<RpmInfo> {
    let mut f = line.split('\t');
    let (name, version) = (f.next()?, f.next()?);
    if name.contains(' ') || name.is_empty() {
        return None;
    }
    Some((
        name.to_string(),
        version.to_string(),
        f.next().and_then(|s| s.trim().parse().ok()),
    ))
}

/// Name, version, size in bytes.
type RpmInfo = (String, String, Option<u64>);

fn rpm_owners(paths: &[String]) -> Vec<(String, RpmInfo)> {
    const QF: &str = "%{NAME}\t%{VERSION}-%{RELEASE}\t%{SIZE}\n";
    let mut args = vec!["-qf", "--qf", QF];
    args.extend(paths.iter().map(String::as_str));
    let out = output("rpm", &args).unwrap_or_default();
    let lines: Vec<&str> = out.lines().collect();
    // One line per path, unless a file has several owners: then ask singly.
    if lines.len() == paths.len() {
        return paths
            .iter()
            .zip(lines)
            .filter_map(|(p, l)| Some((p.clone(), parse_rpm_line(l)?)))
            .collect();
    }
    paths
        .iter()
        .filter_map(|p| {
            let out = output("rpm", &["-qf", "--qf", QF, p])?;
            Some((p.clone(), parse_rpm_line(out.lines().next()?)?))
        })
        .collect()
}

#[derive(Debug, PartialEq)]
pub struct FlatpakRow {
    pub app_id: String,
    pub name: String,
    pub version: Option<String>,
    pub user: bool,
    pub size: Option<u64>,
}

/// `flatpak list --app --columns=application,name,version,installation,size`
/// (tab separated when not on a terminal).
pub fn parse_flatpak_list(out: &str) -> Vec<FlatpakRow> {
    out.lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.split('\t').map(str::trim).collect();
            let app_id = *f.first()?;
            if !app_id.contains('.') || !valid_name(app_id) {
                return None;
            }
            Some(FlatpakRow {
                app_id: app_id.to_string(),
                name: f
                    .get(1)
                    .filter(|n| !n.is_empty())
                    .unwrap_or(&app_id)
                    .to_string(),
                version: f.get(2).filter(|v| !v.is_empty()).map(|v| v.to_string()),
                user: f.get(3) == Some(&"user"),
                size: f.get(4).and_then(|s| parse_size(s)),
            })
        })
        .collect()
}

fn flatpak_apps(home: &Path) -> Vec<App> {
    if !in_path("flatpak") {
        return Vec::new();
    }
    let out = output(
        "flatpak",
        &[
            "list",
            "--app",
            "--columns=application,name,version,installation,size",
        ],
    )
    .unwrap_or_default();
    parse_flatpak_list(&out)
        .into_iter()
        .map(|r| {
            let scope = if r.user { "user" } else { "system" };
            let source = Source::Flatpak {
                app_id: r.app_id.clone(),
                user: r.user,
            };
            let mut app = base_app(format!("flatpak:{}:{scope}", r.app_id), r.name, source);
            app.version = r.version;
            app.size = r.size;
            app.needs_admin = !r.user;
            app.install_dir = Some(if r.user {
                home.join(".local/share/flatpak/app").join(&r.app_id)
            } else {
                Path::new("/var/lib/flatpak/app").join(&r.app_id)
            });
            app.hints = vec![r.app_id.clone()];
            app.uninstall = uninstall_argv(&app).ok().map(|argv| Command::Argv { argv });
            app
        })
        .collect()
}

#[derive(Debug, PartialEq)]
pub struct SnapRow {
    pub name: String,
    pub version: String,
    pub publisher: Option<String>,
}

/// `snap list`: `Name Version Rev Tracking Publisher Notes`, header first.
pub fn parse_snap_list(out: &str) -> Vec<SnapRow> {
    out.lines()
        .skip(1)
        .filter_map(|l| {
            let f: Vec<&str> = l.split_whitespace().collect();
            let (name, version) = (*f.first()?, *f.get(1)?);
            let publisher = f
                .get(4)
                .map(|p| p.trim_end_matches(['✓', '✪', '*']).to_string())
                .filter(|p| !p.is_empty() && p != "-");
            valid_name(name).then(|| SnapRow {
                name: name.into(),
                version: version.into(),
                publisher,
            })
        })
        .collect()
}

fn snap_apps() -> Vec<App> {
    let dir = Path::new("/var/lib/snapd/desktop/applications");
    if !dir.exists() || !in_path("snap") {
        return Vec::new();
    }
    // Snaps with a launcher only: bases, cores and snapd itself have none.
    let desktops = desktop_files(dir);
    let mut by_snap: BTreeMap<String, Vec<&Desktop>> = BTreeMap::new();
    for d in &desktops {
        let s = stem(&d.path);
        let snap = s.split_once('_').map_or(s.as_str(), |(n, _)| n).to_string();
        by_snap.entry(snap).or_default().push(d);
    }
    let rows = parse_snap_list(&output("snap", &["list"]).unwrap_or_default());
    rows.into_iter()
        .filter_map(|r| {
            let ds = by_snap.get(&r.name)?;
            let name = ds[0].entry.name.clone().unwrap_or_else(|| r.name.clone());
            let mut app = base_app(
                format!("snap:{}", r.name),
                name,
                Source::Snap {
                    name: r.name.clone(),
                },
            );
            app.version = Some(r.version);
            app.publisher = r.publisher;
            app.install_dir = Some(Path::new("/snap").join(&r.name));
            app.hints = hints_for(ds, &r.name);
            app.uninstall = uninstall_argv(&app).ok().map(|argv| Command::Argv { argv });
            Some(app)
        })
        .collect()
}

// --------------------------------------------------------------- uninstall

/// Package, Flatpak and snap names as they reach a command line: no option
/// look-alikes, no spaces or shell characters.
pub fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with('-')
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._+:-".contains(&b))
}

fn check(name: &str) -> Result<&str> {
    if valid_name(name) {
        Ok(name)
    } else {
        Err(Error::Failed(format!("invalid package name {name:?}")))
    }
}

/// The command `uninstall` runs, also shown in the app details.
fn uninstall_argv(app: &App) -> Result<Vec<String>> {
    let root = |argv: &[&str]| -> Vec<String> {
        let mut v: Vec<String> = if is_elevated() {
            vec![]
        } else {
            vec!["pkexec".into()]
        };
        v.extend(argv.iter().map(|s| s.to_string()));
        v
    };
    Ok(match &app.source {
        Source::Package { manager, name } => {
            let name = check(name)?;
            match manager {
                PackageManager::Dpkg if in_path("apt-get") => {
                    root(&["apt-get", "remove", "-y", name])
                }
                PackageManager::Dpkg => root(&["dpkg", "-r", name]),
                PackageManager::Pacman => root(&["pacman", "-R", "--noconfirm", name]),
                PackageManager::Rpm if in_path("dnf") => root(&["dnf", "remove", "-y", name]),
                PackageManager::Rpm if in_path("zypper") => {
                    root(&["zypper", "--non-interactive", "remove", name])
                }
                PackageManager::Rpm => root(&["rpm", "-e", name]),
            }
        }
        // Flatpak asks polkit itself for system installs.
        Source::Flatpak { app_id, user } => {
            let scope = if *user { "--user" } else { "--system" };
            vec![
                "flatpak".into(),
                "uninstall".into(),
                "-y".into(),
                scope.into(),
                check(app_id)?.into(),
            ]
        }
        Source::Snap { name } => root(&["snap", "remove", check(name)?]),
        _ => return Err(Error::Unsupported),
    })
}

/// pkexec's own exit codes: 126 the user dismissed the prompt, 127 not
/// authorised.
fn exit_result(program: &str, status: std::process::ExitStatus, via_pkexec: bool) -> Result<i32> {
    match status.code() {
        Some(126 | 127) if via_pkexec => Err(Error::Cancelled),
        Some(code) => Ok(code),
        None => Err(Error::Failed(format!("{program} was stopped by a signal"))),
    }
}

pub fn uninstall(app: &App, _quiet: bool) -> Result<()> {
    // Every command above is already unattended.
    let argv = uninstall_argv(app)?;
    let status = Process::new(&argv[0])
        .args(&argv[1..])
        .status()
        .map_err(|e| Error::Failed(format!("{}: {e}", argv[0])))?;
    match exit_result(&argv[0], status, argv[0] == "pkexec")? {
        0 => Ok(()),
        code => Err(Error::Failed(format!(
            "{} exited with code {code}",
            argv.join(" ")
        ))),
    }
}

// ---------------------------------------------------------------- leftovers

pub fn leftover_roots(level: ScanLevel) -> Vec<Root> {
    let home = home();
    let user = |p: &str, depth: usize| Root {
        path: home.join(p),
        depth,
        needs_admin: false,
    };
    let config_depth = if level == ScanLevel::Deep { 2 } else { 1 };
    let mut roots = vec![
        user(".config", config_depth),
        user(".local/share", 1),
        user(".local/state", 1),
        user(".cache", 1),
        user(".var/app", 1),
        // Only dot entries count here, see scan::skip_in_root.
        Root {
            path: home.clone(),
            depth: 1,
            needs_admin: false,
        },
    ];
    if level == ScanLevel::Deep {
        for p in ["/etc", "/opt", "/var/lib", "/usr/share"] {
            roots.push(Root {
                path: PathBuf::from(p),
                depth: 1,
                needs_admin: true,
            });
        }
    }
    roots
}

pub fn registry_leftovers(_terms: &Terms, _level: ScanLevel, _app: &App) -> Vec<Leftover> {
    Vec::new()
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

pub fn create_restore_point(_description: &str) -> Result<()> {
    Err(Error::Unsupported)
}

// ---------------------------------------------------------------- autostart

const SYSTEM_AUTOSTART: &str = "/etc/xdg/autostart";

fn user_autostart() -> PathBuf {
    home().join(".config/autostart")
}

pub fn autostart_list() -> Result<Vec<AutostartEntry>> {
    let mut files: BTreeMap<String, PathBuf> = BTreeMap::new();
    // The user's file with the same name overrides the system one.
    for dir in [PathBuf::from(SYSTEM_AUTOSTART), user_autostart()] {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for e in entries.filter_map(|e| e.ok()) {
            let name = e.file_name().to_string_lossy().into_owned();
            if name.ends_with(".desktop") {
                files.insert(name, e.path());
            }
        }
    }
    let mut out = Vec::new();
    for (file, path) in files {
        let Ok(text) = fs::read_to_string(&path) else {
            continue;
        };
        let e = parse_desktop(&text);
        // An override that only hides a system entry has no Exec of its own.
        let exec = e.exec.clone().or_else(|| {
            let sys = fs::read_to_string(Path::new(SYSTEM_AUTOSTART).join(&file)).ok()?;
            parse_desktop(&sys).exec
        });
        out.push(AutostartEntry {
            id: format!("xdg:{file}"),
            name: e
                .name
                .clone()
                .unwrap_or_else(|| file.trim_end_matches(".desktop").to_string()),
            command: exec.as_deref().map(exec_display).unwrap_or_default(),
            place: AutostartPlace::Xdg,
            location: path.to_string_lossy().into_owned(),
            enabled: e.is_active(),
            needs_admin: false,
        });
    }
    out.extend(systemd_user_entries());
    out.sort_by_key(|e| e.name.to_lowercase());
    Ok(out)
}

/// `systemctl --user list-unit-files --no-legend`: `unit state preset`.
pub fn parse_unit_files(out: &str) -> Vec<(String, bool)> {
    out.lines()
        .filter_map(|l| {
            let mut f = l.split_whitespace();
            let (unit, state) = (f.next()?, f.next()?);
            // Templates need an instance name to be enabled.
            if unit.contains('@') {
                return None;
            }
            match state {
                "enabled" => Some((unit.to_string(), true)),
                "disabled" => Some((unit.to_string(), false)),
                _ => None,
            }
        })
        .collect()
}

/// `Description=` and `ExecStart=` of a unit file.
pub fn parse_unit(text: &str) -> (Option<String>, Option<String>) {
    let mut desc = None;
    let mut exec = None;
    for line in text.lines().map(str::trim) {
        if let Some(v) = line.strip_prefix("Description=") {
            desc.get_or_insert_with(|| v.trim().to_string());
        } else if let Some(v) = line.strip_prefix("ExecStart=") {
            exec.get_or_insert_with(|| {
                v.trim()
                    .trim_start_matches(['-', '@', '+', '!'])
                    .to_string()
            });
        }
    }
    (desc, exec)
}

fn systemd_user_entries() -> Vec<AutostartEntry> {
    if !in_path("systemctl") {
        return Vec::new();
    }
    let out = output(
        "systemctl",
        &[
            "--user",
            "list-unit-files",
            "--type=service",
            "--state=enabled,disabled",
            "--no-legend",
        ],
    )
    .unwrap_or_default();
    let dirs = [
        home().join(".config/systemd/user"),
        PathBuf::from("/usr/lib/systemd/user"),
    ];
    parse_unit_files(&out)
        .into_iter()
        .filter_map(|(unit, enabled)| {
            let path = dirs.iter().map(|d| d.join(&unit)).find(|p| p.is_file())?;
            let (desc, exec) = parse_unit(&fs::read_to_string(&path).unwrap_or_default());
            Some(AutostartEntry {
                id: format!("systemd-user:{unit}"),
                name: desc.unwrap_or_else(|| unit.clone()),
                command: exec.unwrap_or_default(),
                place: AutostartPlace::SystemdUser,
                location: path.to_string_lossy().into_owned(),
                enabled,
                needs_admin: false,
            })
        })
        .collect()
}

fn valid_file_name(name: &str, suffix: &str) -> bool {
    name.len() > suffix.len()
        && name.ends_with(suffix)
        && !name.starts_with(['.', '-'])
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._+@:-".contains(&b))
}

pub fn autostart_set(entry: &AutostartEntry, enabled: bool) -> Result<()> {
    match entry.place {
        AutostartPlace::Xdg => {
            let file = entry.id.strip_prefix("xdg:").unwrap_or_default();
            if !valid_file_name(file, ".desktop") {
                return Err(Error::Failed(format!("invalid entry {:?}", entry.id)));
            }
            let dir = user_autostart();
            let user = dir.join(file);
            let original = fs::read_to_string(&user)
                .or_else(|_| fs::read_to_string(Path::new(SYSTEM_AUTOSTART).join(file)))
                .ok();
            fs::create_dir_all(&dir)?;
            fs::write(&user, write_override(original.as_deref(), enabled))?;
            Ok(())
        }
        AutostartPlace::SystemdUser => {
            let unit = entry.id.strip_prefix("systemd-user:").unwrap_or_default();
            if !valid_file_name(unit, ".service") {
                return Err(Error::Failed(format!("invalid entry {:?}", entry.id)));
            }
            let verb = if enabled { "enable" } else { "disable" };
            let status = Process::new("systemctl")
                .args(["--user", verb, unit])
                .status()
                .map_err(|e| Error::Failed(format!("systemctl: {e}")))?;
            if status.success() {
                Ok(())
            } else {
                Err(Error::Failed(format!(
                    "systemctl --user {verb} {unit} failed"
                )))
            }
        }
        _ => Err(Error::Unsupported),
    }
}

// --------------------------------------------------------------- elevation

pub fn admin_quarantine_root() -> PathBuf {
    PathBuf::from("/var/lib/uwusweep/quarantine")
}

pub fn is_elevated() -> bool {
    fs::metadata("/proc/self").is_ok_and(|m| m.uid() == 0)
}

pub fn run_elevated(exe: &Path, args: &[String]) -> Result<i32> {
    let mut cmd = if is_elevated() {
        Process::new(exe)
    } else {
        let mut c = Process::new("pkexec");
        c.arg(exe);
        c
    };
    let status = cmd
        .args(args)
        .status()
        .map_err(|e| Error::Failed(format!("pkexec: {e}")))?;
    exit_result("pkexec", status, !is_elevated())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dpkg_parsers() {
        let s = "firefox: /usr/share/applications/firefox.desktop\nlibreoffice-calc, libreoffice-common: /usr/share/applications/libreoffice-calc.desktop\ndiversion by foo from: /usr/share/applications/x.desktop\n";
        let m = parse_dpkg_s(s);
        assert_eq!(m["/usr/share/applications/firefox.desktop"], "firefox");
        assert_eq!(
            m["/usr/share/applications/libreoffice-calc.desktop"],
            "libreoffice-calc"
        );
        assert_eq!(m.len(), 2);
        let q = parse_dpkg_query("firefox\t1:120.0+build1\t251234\nbroken\n");
        assert_eq!(q["firefox"], ("1:120.0+build1".into(), Some(251234 * 1024)));
        assert_eq!(q.len(), 1);
    }

    #[test]
    fn pacman_parsers() {
        let qo = parse_pacman_qo("/usr/share/applications/gimp.desktop is owned by gimp 2.10.36-4\nerror: No package owns /x\n");
        assert_eq!(
            qo,
            [(
                "/usr/share/applications/gimp.desktop".into(),
                "gimp".into(),
                "2.10.36-4".into()
            )]
        );
        let qi = parse_pacman_qi("Name            : gimp\nVersion         : 2.10.36-4\nInstalled Size  : 112.50 MiB\n\nName : vlc\nVersion : 3.0\nInstalled Size : 1024.00 KiB\n");
        assert_eq!(qi["gimp"], ("2.10.36-4".into(), Some(117_964_800)));
        assert_eq!(qi["vlc"].1, Some(1_048_576));
    }

    #[test]
    fn rpm_and_sizes() {
        assert_eq!(
            parse_rpm_line("gimp\t2.10-1.fc39\t12345"),
            Some(("gimp".into(), "2.10-1.fc39".into(), Some(12345)))
        );
        assert_eq!(parse_rpm_line("file /x is not owned by any package"), None);
        assert_eq!(parse_size("1.5 GB"), Some(1_500_000_000));
        assert_eq!(parse_size("3,0 kB"), Some(3000));
        assert_eq!(parse_size("12"), Some(12));
        assert_eq!(parse_size("lots"), None);
    }

    #[test]
    fn flatpak_and_snap_parsers() {
        let rows = parse_flatpak_list("org.example.Editor\tEditor\t1.2.0\tsystem\t250.1 MB\norg.example.Game\t\t\tuser\t1.0 GB\nbad name\tx\n");
        assert_eq!(rows.len(), 2);
        assert_eq!(
            rows[0],
            FlatpakRow {
                app_id: "org.example.Editor".into(),
                name: "Editor".into(),
                version: Some("1.2.0".into()),
                user: false,
                size: Some(250_100_000)
            }
        );
        assert!(rows[1].user && rows[1].name == "org.example.Game" && rows[1].version.is_none());
        let snaps = parse_snap_list("Name      Version   Rev    Tracking       Publisher   Notes\nfirefox   120.0     3358   latest/stable  mozilla✓    -\ncore22    20231123  1033   latest/stable  canonical✓  base\nmytool    1.0       5      latest/stable  -           -\n");
        assert_eq!(snaps.len(), 3);
        assert_eq!(
            snaps[0],
            SnapRow {
                name: "firefox".into(),
                version: "120.0".into(),
                publisher: Some("mozilla".into())
            }
        );
        assert_eq!(snaps[2].publisher, None);
    }

    #[test]
    fn systemd_parsers() {
        let u = parse_unit_files("pipewire.service enabled enabled\nfoo@.service disabled enabled\nbar.service disabled enabled\nbaz.service static -\n");
        assert_eq!(
            u,
            [
                ("pipewire.service".into(), true),
                ("bar.service".into(), false)
            ]
        );
        let (d, e) = parse_unit(
            "[Unit]\nDescription=Sync tool\n[Service]\nExecStart=-/usr/bin/synctool --daemon\n",
        );
        assert_eq!(d.as_deref(), Some("Sync tool"));
        assert_eq!(e.as_deref(), Some("/usr/bin/synctool --daemon"));
    }

    #[test]
    fn names_are_checked() {
        assert!(
            valid_name("libreoffice-calc")
                && valid_name("g++")
                && valid_name("org.example.App")
                && valid_name("pkg:amd64")
        );
        assert!(!valid_name("-rf") && !valid_name("a b") && !valid_name("a;b") && !valid_name(""));
        assert!(valid_file_name("tool.desktop", ".desktop"));
        assert!(
            !valid_file_name("../x.desktop", ".desktop")
                && !valid_file_name(".desktop", ".desktop")
        );
        let mut app = base_app(
            "x".into(),
            "x".into(),
            Source::Package {
                manager: PackageManager::Pacman,
                name: "--all".into(),
            },
        );
        assert!(uninstall_argv(&app).is_err());
        app.source = Source::Flatpak {
            app_id: "org.example.App".into(),
            user: true,
        };
        assert_eq!(
            uninstall_argv(&app).unwrap(),
            ["flatpak", "uninstall", "-y", "--user", "org.example.App"]
        );
    }

    #[test]
    fn roots() {
        let safe = leftover_roots(ScanLevel::Safe);
        let deep = leftover_roots(ScanLevel::Deep);
        assert!(safe.iter().all(|r| !r.needs_admin));
        assert!(deep
            .iter()
            .any(|r| r.path == Path::new("/etc") && r.needs_admin));
        assert!(deep
            .iter()
            .any(|r| r.path.ends_with(".config") && r.depth == 2));
    }

    #[test]
    #[ignore = "reads the real system"]
    fn real_system() {
        let apps = list_apps().unwrap();
        for a in apps.iter().take(5) {
            eprintln!(
                "{} {} {:?} {:?} {:?}",
                a.id, a.name, a.version, a.size, a.hints
            );
        }
        eprintln!(
            "{} apps, {} autostart",
            apps.len(),
            autostart_list().unwrap().len()
        );
        assert!(!apps.is_empty());
    }
}
