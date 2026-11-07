//! The pure half of the macOS module: turning `Info.plist`s, launchd plists
//! and `launchctl`/`osascript` output into data. Kept apart from the system
//! calls so it is tested on every system.

use std::collections::HashMap;
use std::path::Path;

use plist::{Dictionary, Value};

use crate::{App, Source};

/// Apple apps that come from the App Store and can be removed like any other
/// app, even without a receipt (bundled with new Macs).
const REMOVABLE_APPLE: &[&str] = &["com.apple.garageband10", "com.apple.iMovieApp"];

/// Apple's own apps are protected by SIP, except the App Store ones.
pub fn is_removable(bundle_id: Option<&str>, has_receipt: bool) -> bool {
    match bundle_id {
        Some(id) if id.starts_with("com.apple.") => {
            has_receipt || id.starts_with("com.apple.iWork.") || REMOVABLE_APPLE.contains(&id)
        }
        _ => true,
    }
}

/// An app from its bundle path and parsed `Info.plist`. `None` for Apple apps
/// that cannot be removed.
pub fn app_from_info(
    bundle: &Path,
    info: &Dictionary,
    has_receipt: bool,
    installed_on: Option<String>,
    needs_admin: bool,
) -> Option<App> {
    let s = |k: &str| {
        info.get(k)
            .and_then(Value::as_string)
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(str::to_owned)
    };
    let bundle_id = s("CFBundleIdentifier");
    if !is_removable(bundle_id.as_deref(), has_receipt) {
        return None;
    }
    let file_name = bundle.file_stem().map(|n| n.to_string_lossy().into_owned());
    let name = s("CFBundleDisplayName")
        .or_else(|| s("CFBundleName"))
        .or(file_name)?;

    let mut hints = Vec::new();
    if let Some(id) = &bundle_id {
        hints.push(id.clone());
        if let Some(last) = id.rsplit('.').next() {
            hints.push(last.to_owned());
        }
    }
    if let Some(exe) = s("CFBundleExecutable") {
        hints.push(exe);
    }
    let mut seen = std::collections::HashSet::new();
    hints.retain(|h| seen.insert(h.to_lowercase()));

    Some(App {
        id: format!(
            "mac:{}",
            bundle_id
                .clone()
                .unwrap_or_else(|| bundle.display().to_string())
        ),
        name,
        publisher: s("NSHumanReadableCopyright").and_then(|c| publisher_from_copyright(&c)),
        version: s("CFBundleShortVersionString"),
        install_dir: Some(bundle.to_path_buf()),
        size: None,
        installed_on,
        source: Source::MacApp {
            bundle_path: bundle.to_path_buf(),
            bundle_id,
        },
        uninstall: None,
        quiet_uninstall: None,
        winget_id: None,
        needs_admin,
        hints,
    })
}

/// `Copyright © 2020-2024 Example Inc. All rights reserved.` → `Example Inc`.
/// A rough guess; `None` when nothing name-like is left.
pub fn publisher_from_copyright(c: &str) -> Option<String> {
    let mut s = c.lines().next().unwrap_or("").to_string();
    if let Some(i) = s.to_ascii_lowercase().find("all rights reserved") {
        s.truncate(i);
    }
    let words: Vec<&str> = s
        .split_whitespace()
        .filter(|w| {
            let l = w.to_lowercase();
            !(l.starts_with("copyright")
                || l == "©"
                || l == "(c)"
                || l == "by"
                || w.trim_matches(|c: char| !c.is_alphanumeric())
                    .chars()
                    .all(|c| c.is_ascii_digit() || c == '-' || c == '–'))
        })
        .collect();
    let out = words.join(" ");
    let out = out.trim_matches(|c: char| c == '.' || c == ',' || c == ';' || c.is_whitespace());
    (!out.is_empty()).then(|| out.to_owned())
}

/// Seconds since the epoch → `YYYY-MM-DD` (UTC), without a date crate.
pub fn ymd(secs: u64) -> String {
    // Howard Hinnant's days-to-civil.
    let z = (secs / 86_400) as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

/// What a launchd plist says about itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchJob {
    pub label: Option<String>,
    pub command: String,
    pub disabled: bool,
}

pub fn launch_job(plist: &Dictionary) -> LaunchJob {
    let label = plist
        .get("Label")
        .and_then(Value::as_string)
        .map(str::to_owned);
    let args: Vec<&str> = plist
        .get("ProgramArguments")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_string).collect())
        .unwrap_or_default();
    let command = match plist.get("Program").and_then(Value::as_string) {
        // With both, Program is the executable and ProgramArguments[0] argv[0].
        Some(p) if args.len() > 1 => format!("{p} {}", args[1..].join(" ")),
        Some(p) => p.to_owned(),
        None => args.join(" "),
    };
    let disabled = plist
        .get("Disabled")
        .and_then(Value::as_boolean)
        .unwrap_or(false);
    LaunchJob {
        label,
        command,
        disabled,
    }
}

/// `launchctl print-disabled <domain>`: label → disabled. Newer macOS prints
/// `"label" => disabled|enabled`, older `"label" => true|false` (true meaning
/// disabled).
pub fn parse_print_disabled(out: &str) -> HashMap<String, bool> {
    out.lines()
        .filter_map(|l| {
            let (k, v) = l.split_once("=>")?;
            let k = k.trim().trim_matches('"');
            let disabled = match v.trim() {
                "disabled" | "true" => true,
                "enabled" | "false" => false,
                _ => return None,
            };
            (!k.is_empty()).then(|| (k.to_owned(), disabled))
        })
        .collect()
}

/// One argument for `/bin/sh`, single-quoted.
pub fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// The AppleScript that runs `exe args…` as root behind the admin prompt.
pub fn admin_script(exe: &str, args: &[String]) -> String {
    let cmd = std::iter::once(exe)
        .chain(args.iter().map(String::as_str))
        .map(shell_quote)
        .collect::<Vec<_>>()
        .join(" ");
    let escaped = cmd.replace('\\', r"\\").replace('"', "\\\"");
    format!("do shell script \"{escaped}\" with administrator privileges")
}

/// The error code at the end of osascript's message, e.g.
/// `execution error: User canceled. (-128)` → -128, a failing command's exit
/// status otherwise. `None` when there is none.
pub fn osascript_error_code(stderr: &str) -> Option<i32> {
    let s = stderr.trim_end();
    let s = s.strip_suffix(')')?;
    s[s.rfind('(')? + 1..].parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dict(xml: &str) -> Dictionary {
        let doc = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?><plist version="1.0"><dict>{xml}</dict></plist>"#
        );
        Value::from_reader_xml(doc.as_bytes())
            .unwrap()
            .into_dictionary()
            .unwrap()
    }

    #[test]
    fn macos_app_from_info() {
        let info = dict(
            "<key>CFBundleIdentifier</key><string>com.example.Widget</string>\
             <key>CFBundleName</key><string>Widget</string>\
             <key>CFBundleDisplayName</key><string>Widget Pro</string>\
             <key>CFBundleExecutable</key><string>widget</string>\
             <key>CFBundleShortVersionString</key><string>2.1</string>\
             <key>NSHumanReadableCopyright</key><string>Copyright © 2019–2024 Example Inc. All rights reserved.</string>",
        );
        let path = Path::new("/Applications/Widget.app");
        let app = app_from_info(path, &info, false, Some("2024-01-02".into()), true).unwrap();
        assert_eq!(app.id, "mac:com.example.Widget");
        assert_eq!(app.name, "Widget Pro");
        assert_eq!(app.version.as_deref(), Some("2.1"));
        assert_eq!(app.publisher.as_deref(), Some("Example Inc"));
        assert_eq!(app.hints, ["com.example.Widget", "Widget"]);
        assert!(app.needs_admin);
        assert_eq!(app.install_dir.as_deref(), Some(path));

        // No id, no names: falls back to the file name and the path.
        let app = app_from_info(path, &Dictionary::new(), false, None, false).unwrap();
        assert_eq!(
            (app.id.as_str(), app.name.as_str()),
            ("mac:/Applications/Widget.app", "Widget")
        );
    }

    #[test]
    fn macos_apple_apps() {
        let apple = dict("<key>CFBundleIdentifier</key><string>com.apple.Safari</string>");
        assert!(app_from_info(
            Path::new("/Applications/Safari.app"),
            &apple,
            false,
            None,
            true
        )
        .is_none());
        assert!(
            app_from_info(Path::new("/Applications/X.app"), &apple, true, None, true).is_some()
        );
        assert!(is_removable(Some("com.apple.iWork.Pages"), false));
        assert!(is_removable(Some("com.apple.garageband10"), false));
        assert!(!is_removable(Some("com.apple.Terminal"), false));
        assert!(is_removable(Some("com.example.apple"), false));
    }

    #[test]
    fn macos_copyright() {
        assert_eq!(
            publisher_from_copyright("© 2024 Example LLC").as_deref(),
            Some("Example LLC")
        );
        assert_eq!(
            publisher_from_copyright("Copyright (c) 2001-2023, Example Corp.").as_deref(),
            Some("Example Corp")
        );
        assert_eq!(
            publisher_from_copyright("Copyright 2024. All rights reserved."),
            None
        );
        // Lower-casing `İ` changes its length; must not panic.
        assert!(publisher_from_copyright("İİ Example AG. All rights reserved.").is_some());
    }

    #[test]
    fn macos_ymd() {
        assert_eq!(ymd(0), "1970-01-01");
        assert_eq!(ymd(951_782_400), "2000-02-29");
        assert_eq!(ymd(1_709_164_800), "2024-02-29");
        assert_eq!(ymd(1_735_689_599), "2024-12-31");
        assert_eq!(ymd(4_102_444_800), "2100-01-01");
    }

    #[test]
    fn macos_launch_job() {
        let j = launch_job(&dict(
            "<key>Label</key><string>com.example.agent</string>\
             <key>ProgramArguments</key><array><string>/opt/agent</string><string>--bg</string></array>\
             <key>Disabled</key><true/>",
        ));
        assert_eq!(
            j,
            LaunchJob {
                label: Some("com.example.agent".into()),
                command: "/opt/agent --bg".into(),
                disabled: true
            }
        );
        let j = launch_job(&dict("<key>Program</key><string>/opt/a</string>"));
        assert_eq!(
            (j.label, j.command.as_str(), j.disabled),
            (None, "/opt/a", false)
        );
    }

    #[test]
    fn macos_print_disabled() {
        let new = "disabled services = {\n\t\"com.example.a\" => disabled\n\t\"com.example.b\" => enabled\n}\n";
        let old =
            "disabled services = {\n\t\"com.example.a\" => true\n\t\"com.example.b\" => false\n}\n";
        for out in [new, old] {
            let m = parse_print_disabled(out);
            assert_eq!(m.len(), 2);
            assert!(m["com.example.a"]);
            assert!(!m["com.example.b"]);
        }
    }

    #[test]
    fn macos_admin_script() {
        let s = admin_script(
            "/Applications/Uwu Sweep.app/x",
            &["it's".into(), r#"a"b\c"#.into()],
        );
        assert_eq!(
            s,
            r#"do shell script "'/Applications/Uwu Sweep.app/x' 'it'\\''s' 'a\"b\\c'" with administrator privileges"#
        );
        assert_eq!(
            osascript_error_code("0:42: execution error: User canceled. (-128)\n"),
            Some(-128)
        );
        assert_eq!(osascript_error_code("execution error: boom (3)"), Some(3));
        assert_eq!(osascript_error_code("garbage"), None);
    }
}
