//! The leftover search over the file system, plus what the platform adds.

use std::path::{Path, PathBuf};

use walkdir::WalkDir;

use crate::terms::Terms;
use crate::{platform, App, Confidence, ItemKind, Leftover, ScanLevel, Trace};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Root {
    pub path: PathBuf,
    pub depth: usize,
    pub needs_admin: bool,
}

/// The user's home folder.
pub fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .filter(|h| !h.is_empty())
        .map(PathBuf::from)
}

/// In the home folder itself only dot folders belong to programs; the rest
/// is the user's own (`~/Zoom` holds recordings, not settings).
pub(crate) fn skip_in_root(root: &Path, name: &str, home: Option<&Path>) -> bool {
    home == Some(root) && !name.starts_with('.')
}

pub fn scan_roots(roots: &[Root], terms: &Terms, level: ScanLevel) -> Vec<Leftover> {
    let home = home_dir();
    let mut out = Vec::new();
    for root in roots {
        let mut it = WalkDir::new(&root.path)
            .min_depth(1)
            .max_depth(root.depth)
            .follow_links(false)
            .into_iter();
        while let Some(entry) = it.next() {
            // Unreadable folders (permissions) are skipped silently.
            let Ok(entry) = entry else { continue };
            let name = entry.file_name().to_string_lossy();
            if entry.depth() == 1 && skip_in_root(&root.path, &name, home.as_deref()) {
                if entry.file_type().is_dir() {
                    it.skip_current_dir();
                }
                continue;
            }
            let Some((confidence, term)) = terms.matches(&name, level) else {
                continue;
            };
            let is_dir = entry.file_type().is_dir();
            if is_dir {
                // The whole folder goes; what is inside needs no listing.
                it.skip_current_dir();
            }
            out.push(Leftover {
                kind: if is_dir {
                    ItemKind::Dir
                } else {
                    ItemKind::File
                },
                path: entry.path().to_string_lossy().into_owned(),
                value: None,
                size: None,
                confidence,
                reason: term,
                needs_admin: root.needs_admin,
            });
        }
    }
    out
}

pub fn find_leftovers(app: &App, level: ScanLevel, traces: &[Trace]) -> Vec<Leftover> {
    let terms = Terms::for_app(app);
    let mut found = Vec::new();
    if !terms.is_empty() {
        found.extend(scan_roots(&platform::leftover_roots(level), &terms, level));
        found.extend(platform::registry_leftovers(&terms, level, app));
    }
    if let Some(dir) = app.install_dir.as_deref() {
        if let Ok(meta) = dir.symlink_metadata() {
            found.push(Leftover {
                kind: if meta.is_dir() {
                    ItemKind::Dir
                } else {
                    ItemKind::File
                },
                path: dir.to_string_lossy().into_owned(),
                value: None,
                size: None,
                confidence: Confidence::High,
                reason: "installFolder".into(),
                needs_admin: app.needs_admin,
            });
        }
    }
    for trace in traces.iter().filter(|t| t.app_ids.contains(&app.id)) {
        for item in &trace.items {
            let is_file = matches!(item.kind, ItemKind::File | ItemKind::Dir);
            // Registry items are checked by the platform when they are moved.
            if is_file && Path::new(&item.path).symlink_metadata().is_err() {
                continue;
            }
            found.push(Leftover {
                confidence: Confidence::High,
                reason: "trace".into(),
                ..item.clone()
            });
        }
    }
    let mut out = tidy(found);
    for item in &mut out {
        if matches!(item.kind, ItemKind::File | ItemKind::Dir) {
            item.size = Some(dir_size(Path::new(&item.path)));
        }
    }
    out
}

/// Drops protected paths and duplicates (keeping the higher confidence), and
/// items inside a folder or key that is itself listed with at least the same
/// confidence.
fn tidy(mut found: Vec<Leftover>) -> Vec<Leftover> {
    found.retain(|l| match l.kind {
        ItemKind::File | ItemKind::Dir => !is_protected(Path::new(&l.path)),
        ItemKind::RegKey => !is_protected_key(&l.path),
        ItemKind::RegValue => true,
    });
    found.sort_by(|a, b| (&a.path, &a.value, a.confidence).cmp(&(&b.path, &b.value, b.confidence)));
    found.dedup_by(|b, a| a.path == b.path && a.value == b.value);
    let parents: Vec<(String, Confidence, ItemKind)> = found
        .iter()
        .filter(|l| matches!(l.kind, ItemKind::Dir | ItemKind::RegKey))
        .map(|l| (l.path.clone(), l.confidence, l.kind))
        .collect();
    found.retain(|l| {
        !parents.iter().any(|(p, c, k)| {
            *c <= l.confidence
                && (is_under(&l.path, p)
                    || (*k == ItemKind::RegKey && l.kind == ItemKind::RegValue && l.path == *p))
        })
    });
    found
}

/// Is `child` strictly below `parent`, for `/` and `\` separated paths?
pub fn is_under(child: &str, parent: &str) -> bool {
    let parent = parent.trim_end_matches(['/', '\\']);
    child.len() > parent.len()
        && child.starts_with(parent)
        && matches!(child.as_bytes()[parent.len()], b'/' | b'\\')
}

pub fn dir_size(path: &Path) -> u64 {
    WalkDir::new(path)
        .follow_links(false)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter_map(|e| e.metadata().ok())
        .filter(|m| !m.is_dir())
        .map(|m| m.len())
        .sum()
}

/// Folders that must never be moved, wherever they show up (`*` is one
/// path component, `**` everything below). Lower case; compared ignoring case.
const PROTECTED: &[&str] = &[
    // Linux and the Unix part of macOS.
    "usr/*",
    "usr/local/*",
    "usr/share/applications",
    "usr/share/icons",
    "usr/share/doc",
    "usr/share/man",
    "usr/share/locale",
    "usr/share/fonts",
    "usr/share/mime",
    "usr/share/pixmaps",
    "usr/share/dbus-1",
    "usr/share/polkit-1",
    "usr/share/glib-2.0",
    "usr/share/metainfo",
    "usr/share/themes",
    "usr/lib/*",
    "usr/lib/systemd/**",
    "etc/xdg",
    "etc/xdg/autostart",
    "etc/systemd/**",
    "etc/apt/**",
    "etc/dpkg/**",
    "etc/pacman.d",
    "etc/yum.repos.d",
    "etc/ssh",
    "etc/ssl",
    "etc/pam.d",
    "etc/sudoers",
    "etc/sudoers.d",
    "etc/default",
    "etc/init.d",
    "etc/x11",
    "etc/dbus-1",
    "etc/polkit-1",
    "etc/fonts",
    "etc/profile.d",
    "etc/security",
    "etc/alternatives",
    "etc/cron.d",
    "etc/environment.d",
    "etc/udev",
    "etc/modprobe.d",
    "etc/sysctl.d",
    "etc/ld.so.conf.d",
    "etc/networkmanager",
    "etc/passwd",
    "etc/shadow",
    "etc/group",
    "var/*",
    "var/lib/dpkg/**",
    "var/lib/apt/**",
    "var/lib/pacman/**",
    "var/lib/rpm/**",
    "var/lib/flatpak",
    "var/lib/flatpak/exports/**",
    "var/lib/snapd/**",
    "var/lib/systemd/**",
    "var/lib/dbus",
    "var/lib/polkit-1",
    "proc/**",
    "sys/**",
    "dev/**",
    "boot/**",
    "snap",
    // macOS.
    "system/**",
    "library/*",
    "private/**",
    "applications/utilities",
    "volumes/*",
    // Windows (the drive letter is dropped first).
    "windows/*",
    "windows/system32/**",
    "windows/syswow64/**",
    "windows/winsxs/**",
    "program files/common files",
    "program files/windowsapps",
    "program files (x86)/common files",
    "programdata/microsoft",
    "programdata/microsoft/windows/**",
    "programdata/packages",
];

/// The same below a home folder (`/home/<x>`, `/Users/<x>`, `C:\Users\<x>`,
/// `/root`).
const PROTECTED_IN_HOME: &[&str] = &[
    "desktop",
    "documents",
    "downloads",
    "pictures",
    "music",
    "videos",
    "movies",
    "public",
    "templates",
    "applications",
    "library",
    "appdata",
    "onedrive",
    "snap",
    ".config",
    ".config/autostart",
    ".config/systemd",
    ".config/systemd/user",
    ".local",
    ".local/*",
    ".local/share/applications",
    ".local/share/flatpak",
    ".local/share/icons",
    ".local/share/fonts",
    ".local/share/keyrings",
    ".local/share/trash",
    ".cache",
    ".var",
    ".var/app",
    ".ssh",
    ".gnupg",
    ".pki",
    "library/*",
    "appdata/*",
    "appdata/local/programs",
    "appdata/local/temp",
    "appdata/local/microsoft",
    "appdata/local/packages",
    "appdata/roaming/microsoft",
    "appdata/roaming/microsoft/windows/**",
];

/// System folders, home folders and their standard subfolders, the roots
/// themselves, and anything that is not a plain absolute path.
pub fn is_protected(path: &Path) -> bool {
    is_protected_str(&path.to_string_lossy())
}

fn is_protected_str(path: &str) -> bool {
    let b = path.as_bytes();
    let sep = |c: u8| matches!(c, b'/' | b'\\');
    // UNC, `\\?\` and `\\.\` paths name the same folders in a form the
    // patterns below would not recognise.
    if b.len() >= 2 && sep(b[0]) && sep(b[1]) {
        return true;
    }
    // `C:foo` is relative to the drive's current folder, not absolute.
    let drive = b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && sep(b[2]);
    let absolute = (!b.is_empty() && sep(b[0])) || drive;
    // Windows ignores trailing dots and spaces (`Windows.` is `Windows`),
    // short names (`PROGRA~1`) and `:` streams name the same files under
    // another spelling.
    let win = cfg!(windows) || drive;
    let mut parts: Vec<String> = path
        .split(['/', '\\'])
        .filter(|p| !p.is_empty() && *p != ".")
        .map(|p| {
            let p = p.to_lowercase();
            if win {
                p.trim_end_matches(['.', ' ']).to_string()
            } else {
                p
            }
        })
        .collect();
    if drive {
        parts.remove(0);
    }
    let odd = |p: &String| {
        p == ".."
            || p.is_empty()
            || (win
                && (p.contains(':')
                    || p.as_bytes()
                        .windows(2)
                        .any(|w| w[0] == b'~' && w[1].is_ascii_digit())))
    };
    if !absolute || parts.iter().any(odd) {
        return true;
    }
    if parts.len() <= 1 {
        return true;
    }
    if PROTECTED.iter().any(|pat| pattern_matches(pat, &parts)) {
        return true;
    }
    let mut homes: Vec<usize> = Vec::new();
    if parts[0] == "root" {
        homes.push(1);
    }
    if matches!(parts[0].as_str(), "home" | "users") {
        homes.push(2);
    }
    if let Some(home) = home_dir() {
        let home = home.to_string_lossy().to_lowercase();
        let hp: Vec<&str> = home
            .split(['/', '\\'])
            .filter(|p| !p.is_empty() && !p.ends_with(':'))
            .collect();
        if parts.len() >= hp.len() && parts.iter().zip(&hp).all(|(a, b)| a == b) {
            homes.push(hp.len());
        }
    }
    homes.into_iter().any(|h| {
        let rel = &parts[h.min(parts.len())..];
        rel.is_empty()
            || PROTECTED_IN_HOME
                .iter()
                .any(|pat| pattern_matches(pat, rel))
    })
}

/// `*` matches one component, a trailing `**` the folder and all below it.
fn pattern_matches(pattern: &str, parts: &[String]) -> bool {
    let mut pat: Vec<&str> = pattern.split('/').collect();
    if pat.last() == Some(&"**") {
        pat.pop();
        return parts.len() >= pat.len() && pat.iter().zip(parts).all(|(p, s)| *p == "*" || p == s);
    }
    parts.len() == pat.len() && pat.iter().zip(parts).all(|(p, s)| *p == "*" || p == s)
}

/// Registry keys that must never be removed: hives, `Software` itself and
/// the system's own branches.
pub fn is_protected_key(key: &str) -> bool {
    let parts: Vec<String> = key
        .split('\\')
        .filter(|p| !p.is_empty())
        .map(|p| p.to_lowercase())
        .collect();
    if parts.len() <= 2 {
        return true;
    }
    let rest = &parts[1..];
    const KEYS: &[&str] = &[
        "software/*",
        "software/microsoft/**",
        "software/wow6432node/*",
        "software/wow6432node/microsoft/**",
        "software/classes/*",
        "software/policies/**",
        "system/**",
        "sam/**",
        "security/**",
    ];
    // Apps' own keys below Software\Microsoft (Uninstall entries, Run values)
    // are removed as values or by the platform, never as whole keys.
    KEYS.iter().any(|pat| pattern_matches(pat, rest)) && !is_app_key_below_software(rest)
}

fn is_app_key_below_software(rest: &[String]) -> bool {
    // Software\Vendor\App and Software\Classes\<progid> stay removable.
    rest.len() >= 3
        && rest[0] == "software"
        && !matches!(
            rest[1].as_str(),
            "microsoft" | "wow6432node" | "policies" | "classes"
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{App, Source};

    #[test]
    fn protected_paths() {
        for p in [
            "/",
            "/usr",
            "/etc",
            "/bin",
            "/lib64",
            "/opt",
            "/var",
            "/var/lib",
            "/usr/share",
            "/usr/share/applications",
            "/home/someone",
            "/home/someone/.config",
            "/home/someone/.local",
            "/home/someone/.local/share",
            "/home/someone/.cache",
            "/home/someone/Desktop",
            "/home/someone/Documents",
            "/home/someone/Downloads",
            "/root",
            "/root/.config",
            "/Users/someone/Library",
            "/Users/someone/Library/Preferences",
            "/Users/someone/Library/Application Support",
            "/Applications",
            "/System/Library/x",
            "/Library",
            "/Library/Application Support",
            r"C:\",
            r"C:\Windows",
            r"C:\Windows\Fonts",
            r"C:\Windows\System32\drivers\x.sys",
            r"C:\Program Files",
            r"C:\Program Files (x86)",
            r"C:\ProgramData",
            r"C:\Users\someone",
            r"C:\Users\someone\AppData\Roaming",
            r"C:\Users\someone\AppData\Local",
            r"C:\Users\someone\AppData\LocalLow",
            r"C:\Users\someone\Documents",
            "relative/path",
            "/opt/../etc",
            "/etc/sudoers.d",
            r"\\?\C:\Windows\System32",
            r"\\server\share\x\y",
            r"//server/share/x/y",
            r"C:Windows\System32",
            r"C:\Windows.\System32",
            r"C:\Windows \System32\x",
            r"C:\PROGRA~1\COMMON~1",
            r"C:\ProgramData\SomeApp:stream",
            r"C:\Users\someone\Documents\..",
        ] {
            assert!(is_protected(Path::new(p)), "{p} should be protected");
        }
        for p in [
            "/opt/someapp",
            "/etc/someapp",
            "/usr/share/someapp",
            "/var/lib/someapp",
            "/home/someone/.config/someapp",
            "/home/someone/.local/share/someapp",
            "/home/someone/.someapp",
            "/Users/someone/Library/Application Support/SomeApp",
            "/Users/someone/Library/Preferences/org.example.app.plist",
            "/Applications/Some.app",
            r"C:\Program Files\SomeApp",
            r"C:\Users\someone\AppData\Roaming\SomeApp",
            r"C:\ProgramData\SomeApp",
            "/home/someone/Documents/Some App/x",
        ] {
            assert!(!is_protected(Path::new(p)), "{p} should not be protected");
        }
    }

    #[test]
    fn protected_keys() {
        for k in [
            r"HKCU",
            r"HKCU\Software",
            r"HKLM\Software\Microsoft",
            r"HKLM\SOFTWARE\Microsoft\Windows",
            r"HKLM\Software\WOW6432Node",
            r"HKLM\System\CurrentControlSet",
            r"HKCU\Software\Vendor",
        ] {
            assert!(is_protected_key(k), "{k}");
        }
        for k in [
            r"HKCU\Software\Vendor\App",
            r"HKLM\Software\WOW6432Node\Vendor\App",
        ] {
            assert!(!is_protected_key(k), "{k}");
        }
    }

    fn app(name: &str) -> App {
        App {
            id: "t:1".into(),
            name: name.into(),
            publisher: None,
            version: None,
            install_dir: None,
            size: None,
            installed_on: None,
            source: Source::Snap { name: "x".into() },
            uninstall: None,
            quiet_uninstall: None,
            winget_id: None,
            needs_admin: false,
            hints: vec![],
        }
    }

    #[test]
    fn scans_without_descending_into_matches() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();
        std::fs::create_dir_all(base.join("Widgetor/Widgetor")).unwrap();
        std::fs::write(base.join("Widgetor/Widgetor/a"), "12345").unwrap();
        std::fs::create_dir_all(base.join("other/deeper")).unwrap();
        std::fs::write(base.join("other/widgetor.conf"), "x").unwrap();
        std::fs::write(base.join("other/widgetor-cache"), "x").unwrap();
        let roots = [Root {
            path: base.to_path_buf(),
            depth: 2,
            needs_admin: false,
        }];
        let terms = Terms::for_app(&app("Widgetor"));
        let mut safe = scan_roots(&roots, &terms, ScanLevel::Safe);
        safe.sort_by(|a, b| a.path.cmp(&b.path));
        assert_eq!(safe.len(), 2, "{safe:?}");
        assert_eq!(safe[0].kind, ItemKind::Dir);
        assert!(safe[0].path.ends_with("Widgetor"));
        assert!(safe[1].path.ends_with("widgetor.conf"));
        let deep = scan_roots(&roots, &terms, ScanLevel::Deep);
        assert_eq!(deep.len(), 3);
        assert_eq!(dir_size(&base.join("Widgetor")), 5);
    }

    fn lo(path: &str, kind: ItemKind, confidence: Confidence) -> Leftover {
        Leftover {
            kind,
            path: path.into(),
            value: None,
            size: None,
            confidence,
            reason: "x".into(),
            needs_admin: false,
        }
    }

    #[test]
    fn tidy_dedups_and_drops_children() {
        let out = tidy(vec![
            lo("/opt/app", ItemKind::Dir, Confidence::High),
            lo("/opt/app/sub", ItemKind::Dir, Confidence::High),
            lo("/opt/app", ItemKind::Dir, Confidence::Medium),
            lo("/opt/app-other", ItemKind::File, Confidence::High),
            lo("/opt/vendor", ItemKind::Dir, Confidence::Medium),
            lo("/opt/vendor/app", ItemKind::Dir, Confidence::High),
            lo("/usr", ItemKind::Dir, Confidence::High),
        ]);
        let paths: Vec<(&str, Confidence)> = out
            .iter()
            .map(|l| (l.path.as_str(), l.confidence))
            .collect();
        assert_eq!(
            paths,
            [
                ("/opt/app", Confidence::High),
                ("/opt/app-other", Confidence::High),
                ("/opt/vendor", Confidence::Medium),
                ("/opt/vendor/app", Confidence::High),
            ]
        );
    }

    #[test]
    fn trace_items_and_install_dir() {
        let dir = tempfile::tempdir().unwrap();
        let install = dir.path().join("SomeInstall");
        std::fs::create_dir(&install).unwrap();
        let kept = dir.path().join("trace-file");
        std::fs::write(&kept, "ab").unwrap();
        let mut a = app("Qqqzzx");
        a.install_dir = Some(install.clone());
        let trace = Trace {
            id: "t".into(),
            name: "t".into(),
            created: 0,
            app_ids: vec![a.id.clone()],
            items: vec![
                lo(&kept.to_string_lossy(), ItemKind::File, Confidence::Medium),
                lo(
                    &dir.path().join("gone").to_string_lossy(),
                    ItemKind::File,
                    Confidence::High,
                ),
                lo(
                    r"HKCU\Software\Vendor\App",
                    ItemKind::RegKey,
                    Confidence::High,
                ),
            ],
        };
        let out = find_leftovers(&a, ScanLevel::Safe, &[trace]);
        let ours: Vec<&Leftover> = out
            .iter()
            .filter(|l| {
                l.path.starts_with(dir.path().to_str().unwrap()) || l.path.starts_with("HKCU")
            })
            .collect();
        assert_eq!(ours.len(), 3, "{out:?}");
        assert!(ours.iter().all(|l| l.confidence == Confidence::High));
        assert!(ours.iter().any(|l| l.reason == "installFolder"));
        assert!(ours
            .iter()
            .any(|l| l.path.ends_with("trace-file") && l.size == Some(2)));
    }
}
