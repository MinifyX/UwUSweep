//! Shared parsing for autostart entries: XDG `.desktop` files (also used for
//! the Linux app list) and the override that switches one off or on.

/// The keys of a `[Desktop Entry]` group that UwUSweep reads.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DesktopEntry {
    pub name: Option<String>,
    pub exec: Option<String>,
    /// `Type=`, usually `Application`.
    pub kind: Option<String>,
    pub hidden: bool,
    pub no_display: bool,
    /// `X-GNOME-Autostart-enabled`, true when missing.
    pub autostart_enabled: bool,
}

impl DesktopEntry {
    /// Does it start at login? (`OnlyShowIn` is ignored: it depends on the
    /// desktop, and turning an entry off works the same everywhere.)
    pub fn is_active(&self) -> bool {
        !self.hidden && self.autostart_enabled
    }
}

fn is_true(v: &str) -> bool {
    v.trim().eq_ignore_ascii_case("true")
}

/// Reads the `[Desktop Entry]` group; localised keys (`Name[de]`) and other
/// groups (`[Desktop Action …]`) are skipped.
pub fn parse_desktop(text: &str) -> DesktopEntry {
    let mut e = DesktopEntry {
        autostart_enabled: true,
        ..Default::default()
    };
    let mut in_main = false;
    for line in text.lines().map(str::trim) {
        if line.starts_with('[') {
            in_main = line == "[Desktop Entry]";
            continue;
        }
        if !in_main || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let value = value.trim();
        match key.trim() {
            "Name" => e.name = Some(value.to_string()),
            "Exec" => e.exec = Some(value.to_string()),
            "Type" => e.kind = Some(value.to_string()),
            "Hidden" => e.hidden = is_true(value),
            "NoDisplay" => e.no_display = is_true(value),
            "X-GNOME-Autostart-enabled" => e.autostart_enabled = is_true(value),
            _ => {}
        }
    }
    e
}

/// `Exec` without the field codes (`%u`, `%F`…), for showing.
pub fn exec_display(exec: &str) -> String {
    exec.split_whitespace()
        .filter(|t| !(t.len() == 2 && t.starts_with('%')))
        .collect::<Vec<_>>()
        .join(" ")
}

/// The program's file name from `Exec`: `env A=b /opt/x/bin/tool --flag` → `tool`.
pub fn exec_program(exec: &str) -> Option<String> {
    let prog = exec
        .split_whitespace()
        .map(|t| t.trim_matches(['"', '\'']))
        .find(|t| *t != "env" && !t.contains('=') && !t.starts_with('-'))?;
    let base = prog.rsplit('/').next()?;
    (!base.is_empty()).then(|| base.to_string())
}

/// The text of a user override: the original entry (when known) with
/// `Hidden` and `X-GNOME-Autostart-enabled` set, so both the spec and GNOME
/// see it.
pub fn write_override(original: Option<&str>, enabled: bool) -> String {
    let text = original.unwrap_or("[Desktop Entry]\nType=Application\n");
    let text = set_key(text, "Hidden", if enabled { "false" } else { "true" });
    set_key(
        &text,
        "X-GNOME-Autostart-enabled",
        if enabled { "true" } else { "false" },
    )
}

/// Sets `key=value` in the `[Desktop Entry]` group, replacing it or adding it
/// at the group's end.
pub fn set_key(text: &str, key: &str, value: &str) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut in_main = false;
    let mut seen_main = false;
    let mut done = false;
    for line in text.lines() {
        let t = line.trim();
        if t.starts_with('[') {
            if in_main && !done {
                out.push(format!("{key}={value}"));
                done = true;
            }
            in_main = t == "[Desktop Entry]";
            seen_main |= in_main;
        } else if in_main && t.split_once('=').is_some_and(|(k, _)| k.trim() == key) {
            if !done {
                out.push(format!("{key}={value}"));
                done = true;
            }
            continue;
        }
        out.push(line.to_string());
    }
    if !done {
        if !seen_main {
            out.insert(0, "[Desktop Entry]".into());
        }
        if in_main || !seen_main {
            out.push(format!("{key}={value}"));
        }
    }
    let mut s = out.join("\n");
    s.push('\n');
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "# comment\n[Desktop Entry]\nType=Application\nName=Some Tool\nName[de]=Ein Werkzeug\nExec=env FOO=1 /opt/some/bin/some-tool --minimized %U\nOnlyShowIn=GNOME;\nX-GNOME-Autostart-enabled=false\n\n[Desktop Action new]\nName=New\nExec=other\n";

    #[test]
    fn parses() {
        let e = parse_desktop(SAMPLE);
        assert_eq!(e.name.as_deref(), Some("Some Tool"));
        assert_eq!(e.kind.as_deref(), Some("Application"));
        assert!(!e.hidden && !e.autostart_enabled && !e.is_active());
        let exec = e.exec.unwrap();
        assert_eq!(
            exec_display(&exec),
            "env FOO=1 /opt/some/bin/some-tool --minimized"
        );
        assert_eq!(exec_program(&exec).as_deref(), Some("some-tool"));
        assert!(parse_desktop("[Desktop Entry]\nHidden=true\n").hidden);
        assert!(parse_desktop("[Desktop Entry]\nName=x\n").is_active());
    }

    #[test]
    fn overrides() {
        let on = write_override(Some(SAMPLE), true);
        let e = parse_desktop(&on);
        assert!(e.is_active(), "{on}");
        assert_eq!(e.name.as_deref(), Some("Some Tool"));
        assert_eq!(on.matches("X-GNOME-Autostart-enabled").count(), 1);
        assert!(on.contains("[Desktop Action new]\nName=New"));
        // The new key lands in the main group, before the action group.
        assert!(on.find("Hidden=false").unwrap() < on.find("[Desktop Action").unwrap());
        let off = write_override(Some(&on), false);
        assert!(parse_desktop(&off).hidden && !parse_desktop(&off).is_active());
        assert!(!parse_desktop(&write_override(None, false)).is_active());
        assert!(parse_desktop(&write_override(Some("Name=odd\n"), true)).is_active());
    }
}
