//! Search words for the leftover search, made from what is known about an app.

use crate::{App, Confidence, ScanLevel};

/// Words that name a vendor, a system or a kind of folder rather than one
/// program. Alone they would match half of `AppData` or `~/.config`.
const GENERIC: &[&str] = &[
    "microsoft",
    "windows",
    "apple",
    "google",
    "inc",
    "gmbh",
    "ltd",
    "llc",
    "corporation",
    "corp",
    "software",
    "app",
    "apps",
    "application",
    "applications",
    "commonfiles",
    "update",
    "updater",
    "updates",
    "helper",
    "helpers",
    "system",
    "user",
    "users",
    "data",
    "cache",
    "caches",
    "temp",
    "tmp",
    "programs",
    "programfiles",
    "programdata",
    "shared",
    "share",
    "local",
    "locallow",
    "roaming",
    "appdata",
    "config",
    "configs",
    "library",
    "preferences",
    "support",
    "desktop",
    "documents",
    "downloads",
    "default",
    "settings",
    "launcher",
    "service",
    "services",
    "tools",
    "utils",
    "client",
    "server",
    "plugin",
    "plugins",
    "extension",
    "extensions",
    "runtime",
    "driver",
    "drivers",
    "bin",
    "lib",
    "var",
    "etc",
    "opt",
    "usr",
    "home",
    "files",
    "file",
    "logs",
    "log",
    "crash",
    "crashes",
    "crashreports",
    "linux",
    "gnome",
    "kde",
    "flatpak",
    "snap",
    "snapd",
    "ubuntu",
    "debian",
    "package",
    "packages",
    "studio",
    "player",
    "editor",
    "viewer",
    "manager",
    "browser",
    "media",
    "office",
    "game",
    "games",
    "state",
    "autostart",
    "systemd",
    "fonts",
    "icons",
    "themes",
    "mime",
    "keyrings",
    "trash",
    "recent",
    "history",
    "storage",
    "backup",
    "backups",
    "profile",
    "profiles",
    "common",
    "install",
    "installer",
    "uninstall",
    "containers",
    "groupcontainers",
    "savedapplicationstate",
    "launchagents",
    "launchdaemons",
    "applicationsupport",
    "cookies",
    "webkit",
    "http",
    "https",
    "www",
    "net",
    "org",
    "com",
    "the",
    "macos",
    "info",
    "help",
    "docs",
    "manual",
    "terminal",
    "python",
    "java",
    "bash",
    "node",
    "electron",
    "wine",
    "mono",
    "perl",
    "ruby",
    "dbus",
    "open",
    "start",
    "mobile",
    "main",
    "core",
    "base",
    "free",
    "lite",
    "beta",
];

/// Company suffixes, dropped from a publisher before it becomes a search word.
const LEGAL: &[&str] = &[
    "inc",
    "llc",
    "ltd",
    "gmbh",
    "ag",
    "co",
    "corp",
    "corporation",
    "limited",
    "sa",
    "srl",
    "bv",
    "kg",
    "plc",
    "ab",
    "oy",
    "pty",
    "the",
    "foundation",
];

/// File endings that do not belong to the name: `org.example.App.plist`
/// should match the bundle id `org.example.App`.
const EXTENSIONS: &[&str] = &[
    "desktop",
    "plist",
    "json",
    "conf",
    "cfg",
    "ini",
    "log",
    "lnk",
    "xml",
    "db",
    "sqlite",
    "lock",
    "pid",
    "savedstate",
    "app",
    "service",
    "list",
    "toml",
    "yaml",
    "yml",
    "binarycookies",
];

/// Shorter words ("app", "mac", the last part of many bundle ids) match
/// too much even exactly.
const MIN_EXACT: usize = 4;
/// "Contains" matches need a longer word, or "code" would match "barcode".
const MIN_CONTAINS: usize = 5;

#[derive(Debug, Clone, Default)]
pub struct Terms {
    /// Words from the name, the install folder and the hints.
    pub words: Vec<String>,
    /// The publisher: its folder often holds other apps of the same vendor,
    /// so it only ever gives a Medium match, and only in a deep scan.
    pub publisher: Option<String>,
    /// Reverse-DNS ids (bundle or Flatpak ids), lower case: macOS and
    /// Flatpak name files `<id>.plist`, `<id>.savedState`, `TEAMID.<id>`.
    pub ids: Vec<String>,
}

impl Terms {
    pub fn for_app(app: &App) -> Terms {
        let mut terms = Terms::default();
        let name = normalize(&app.name);
        let publisher = app.publisher.as_deref().map(normalize_publisher);
        terms.push(name.clone());
        // "Mozilla Firefox" by "Mozilla" also lives in a folder "Firefox".
        if let Some(p) = publisher.as_deref().filter(|p| !p.is_empty()) {
            if let Some(rest) = name.strip_prefix(p) {
                terms.push(rest.to_string());
            }
        }
        if let Some(dir) = app.install_dir.as_deref().and_then(|d| d.file_name()) {
            terms.push(normalize(&dir.to_string_lossy()));
        }
        for hint in &app.hints {
            terms.push(normalize(hint));
            let id = hint.to_lowercase();
            if id.matches('.').count() >= 2 && !id.starts_with('.') && !terms.ids.contains(&id) {
                terms.ids.push(id);
            }
        }
        terms.publisher = publisher.filter(|p| usable(p) && !terms.words.contains(p));
        terms
    }

    fn push(&mut self, word: String) {
        if usable(&word) && !self.words.contains(&word) {
            self.words.push(word);
        }
    }

    /// Does `name` (one file, folder or key name, not a whole path) match?
    pub fn matches(&self, name: &str, level: ScanLevel) -> Option<(Confidence, String)> {
        let raw = strip_team_id(name).to_lowercase();
        for id in &self.ids {
            if raw == *id
                || raw
                    .strip_prefix(id.as_str())
                    .is_some_and(|r| r.starts_with('.'))
            {
                return Some((Confidence::High, id.clone()));
            }
        }
        let mut candidates = vec![normalize(name)];
        if let Some(stem) = strip_extension(name) {
            candidates.push(normalize(stem));
        }
        candidates.retain(|c| !c.is_empty());
        for c in &candidates {
            if let Some(w) = self.words.iter().find(|w| *w == c) {
                return Some((Confidence::High, w.clone()));
            }
        }
        if level == ScanLevel::Deep {
            for c in &candidates {
                if let Some(w) = self
                    .words
                    .iter()
                    .find(|w| w.len() >= MIN_CONTAINS && c.contains(w.as_str()))
                {
                    return Some((Confidence::Medium, w.clone()));
                }
                if let Some(p) = self.publisher.as_ref().filter(|p| *p == c) {
                    return Some((Confidence::Medium, p.clone()));
                }
            }
        }
        None
    }

    pub fn is_empty(&self) -> bool {
        self.words.is_empty() && self.publisher.is_none() && self.ids.is_empty()
    }
}

/// `ABCDE12345.com.example.app` (a macOS group container) → `com.example.app`.
fn strip_team_id(name: &str) -> &str {
    let b = name.as_bytes();
    if b.len() > 11
        && b[10] == b'.'
        && b[..10]
            .iter()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
    {
        &name[11..]
    } else {
        name
    }
}

fn usable(word: &str) -> bool {
    word.len() >= MIN_EXACT && !GENERIC.contains(&word)
}

fn strip_extension(name: &str) -> Option<&str> {
    let (stem, ext) = name.rsplit_once('.')?;
    (!stem.is_empty() && EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str())).then_some(stem)
}

fn normalize_publisher(p: &str) -> String {
    let lower = p.to_lowercase();
    let kept: Vec<&str> = lower
        .split(|c: char| c.is_whitespace() || c == ',')
        .filter(|t| !LEGAL.contains(&t.trim_matches(|c: char| !c.is_alphanumeric())))
        .collect();
    normalize(&kept.join(" "))
}

/// Lower case, only letters and digits, without version numbers and
/// architecture or installer suffixes: `7-Zip 23.01 (x64)` → `7zip`.
pub fn normalize(s: &str) -> String {
    let lower = s.to_lowercase();
    let mut out = String::new();
    let tokens = lower
        .split(|c: char| c.is_whitespace() || matches!(c, '(' | ')' | '[' | ']' | ',' | '_'))
        .map(|t| t.trim_matches(|c: char| !c.is_alphanumeric()))
        .filter(|t| !t.is_empty());
    for (i, token) in tokens.enumerate() {
        if is_noise(token) || (i > 0 && token.chars().all(|c| c.is_ascii_digit())) {
            continue;
        }
        // "app-2.4.1": drop dotted version parts, but keep "7" in "7-zip".
        for part in token.split('-') {
            if part.contains('.') && is_version(part) {
                continue;
            }
            out.extend(part.chars().filter(|c| c.is_alphanumeric()));
        }
    }
    out
}

fn is_noise(t: &str) -> bool {
    matches!(
        t,
        "x64"
            | "x86"
            | "amd64"
            | "arm64"
            | "aarch64"
            | "win32"
            | "win64"
            | "64-bit"
            | "32-bit"
            | "64bit"
            | "32bit"
            | "setup"
    ) || (t.contains('.') && is_version(t))
}

fn is_version(t: &str) -> bool {
    let t = t.strip_prefix('v').unwrap_or(t);
    t.starts_with(|c: char| c.is_ascii_digit()) && t.chars().all(|c| c.is_ascii_digit() || c == '.')
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{App, Source};

    fn app(name: &str, publisher: Option<&str>, hints: &[&str]) -> App {
        App {
            id: "test".into(),
            name: name.into(),
            publisher: publisher.map(Into::into),
            version: None,
            install_dir: None,
            size: None,
            installed_on: None,
            source: Source::Snap {
                name: "test".into(),
            },
            uninstall: None,
            quiet_uninstall: None,
            winget_id: None,
            needs_admin: false,
            hints: hints.iter().map(|h| h.to_string()).collect(),
        }
    }

    #[test]
    fn normalizes() {
        assert_eq!(normalize("7-Zip 23.01 (x64)"), "7zip");
        assert_eq!(normalize("Mozilla Firefox"), "mozillafirefox");
        assert_eq!(normalize(".mozilla"), "mozilla");
        assert_eq!(normalize("Foo App 2.4.1 Setup"), "fooapp");
        assert_eq!(normalize("Office 2019 (64-bit)"), "office");
        assert_eq!(normalize("tool-1.2.3"), "tool");
        assert_eq!(normalize("7 Days to Die"), "7daystodie");
        assert_eq!(normalize("org.example.App"), "orgexampleapp");
        assert_eq!(normalize("v1.2"), "");
    }

    #[test]
    fn match_table() {
        use Confidence::*;
        use ScanLevel::*;
        let ff = Terms::for_app(&app(
            "Mozilla Firefox",
            Some("Mozilla Foundation"),
            &["org.example.Firefox"],
        ));
        let zip = Terms::for_app(&app("7-Zip 23.01 (x64)", Some("Igor Pavlov"), &[]));
        let edge = Terms::for_app(&app("Microsoft Edge", Some("Microsoft Corporation"), &[]));
        let cases: &[(&Terms, &str, ScanLevel, Option<Confidence>)] = &[
            (&ff, "Mozilla Firefox", Safe, Some(High)),
            (&ff, "Firefox", Safe, Some(High)),
            (&ff, ".firefox", Safe, Some(High)),
            (&ff, "Mozilla", Safe, None),
            (&ff, "Mozilla", Deep, Some(Medium)),
            (&ff, "org.example.Firefox.plist", Safe, Some(High)),
            (&ff, "mozillafirefox-backup", Safe, None),
            (&ff, "mozillafirefox-backup", Deep, Some(Medium)),
            (&ff, "Microsoft", Deep, None),
            (&zip, "7-Zip", Safe, Some(High)),
            (&zip, "7zip", Safe, Some(High)),
            (&zip, "Igor Pavlov", Safe, None),
            (&zip, "Igor Pavlov", Deep, Some(Medium)),
            (&edge, "Microsoft", Deep, None),
            (&edge, "Edge", Safe, Some(High)),
            (&edge, "Microsoft Edge", Safe, Some(High)),
        ];
        for (terms, name, level, want) in cases {
            let got = terms.matches(name, *level).map(|(c, _)| c);
            assert_eq!(got, *want, "{name} at {level:?} with {terms:?}");
        }
    }

    #[test]
    fn publisher_medium_only_when_not_generic() {
        let t = Terms::for_app(&app("Thing", Some("Vendorly GmbH"), &[]));
        assert_eq!(t.publisher.as_deref(), Some("vendorly"));
        assert_eq!(
            t.matches("Vendorly", ScanLevel::Deep),
            Some((Confidence::Medium, "vendorly".into()))
        );
        assert_eq!(t.matches("Vendorly", ScanLevel::Safe), None);
    }

    #[test]
    fn generic_and_short_words_are_dropped() {
        let t = Terms::for_app(&app("Helper", Some("Microsoft"), &["go", "data"]));
        assert!(t.is_empty(), "{t:?}");
        assert_eq!(t.matches("helper", ScanLevel::Deep), None);
    }

    #[test]
    fn bundle_ids_match_safe() {
        let t = Terms::for_app(&app("Example", None, &["com.example.app", "app", "mac"]));
        for name in [
            "com.example.app",
            "com.example.app.plist",
            "ABCDE12345.com.example.app",
            "com.example.app.savedState",
            "com.example.app.binarycookies",
            "com.example.app.pkg.bom",
            "com.example.app.pkg.plist",
            "COM.EXAMPLE.APP",
        ] {
            assert_eq!(
                t.matches(name, ScanLevel::Safe).map(|m| m.0),
                Some(Confidence::High),
                "{name}"
            );
        }
        for name in [
            "com.example.application",
            "com.example",
            "app",
            "mac",
            "abcde.com.example.app",
        ] {
            assert_eq!(t.matches(name, ScanLevel::Safe), None, "{name}");
        }
        // The short last part of the id is no search word of its own.
        assert!(!t.words.iter().any(|w| w == "app" || w == "mac"), "{t:?}");
    }

    #[test]
    fn contains_needs_long_word() {
        let t = Terms::for_app(&app("Zedx", None, &[]));
        assert_eq!(
            t.matches("zedx", ScanLevel::Safe).map(|m| m.0),
            Some(Confidence::High)
        );
        assert_eq!(t.matches("zedxboard", ScanLevel::Deep), None);
    }
}
