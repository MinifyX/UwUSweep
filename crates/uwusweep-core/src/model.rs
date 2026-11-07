//! The data UwUSweep shows and acts on. Everything here is serialised to the
//! page as camelCase JSON, and the same shapes go into the helper's job file,
//! the quarantine manifests and the saved traces.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Where an installed app comes from, and so how it is removed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Source {
    /// Windows: a key under `…\CurrentVersion\Uninstall`. `hive` is `HKLM` or
    /// `HKCU`, `key` the full path below the hive, `wow64` the 32-bit view.
    #[serde(rename_all = "camelCase")]
    Registry {
        hive: String,
        key: String,
        wow64: bool,
        /// The MSI product code, when the key is one (`{GUID}`).
        msi_product_code: Option<String>,
    },
    /// Windows: a Microsoft Store / MSIX package.
    #[serde(rename_all = "camelCase")]
    Store {
        package_full_name: String,
        package_family_name: String,
    },
    /// macOS: an `.app` bundle.
    #[serde(rename_all = "camelCase")]
    MacApp {
        bundle_path: PathBuf,
        bundle_id: Option<String>,
    },
    /// Linux: a system package that owns a `.desktop` file.
    #[serde(rename_all = "camelCase")]
    Package {
        manager: PackageManager,
        name: String,
    },
    /// Linux: a Flatpak app, per user or system wide.
    #[serde(rename_all = "camelCase")]
    Flatpak { app_id: String, user: bool },
    /// Linux: a Snap.
    #[serde(rename_all = "camelCase")]
    Snap { name: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PackageManager {
    Dpkg,
    Pacman,
    Rpm,
}

/// One installed program, as the list shows it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct App {
    /// Stable within one listing and across restarts: derived from the source.
    pub id: String,
    pub name: String,
    pub publisher: Option<String>,
    pub version: Option<String>,
    pub install_dir: Option<PathBuf>,
    /// Bytes, when the system knows or it was cheap to count.
    pub size: Option<u64>,
    /// `YYYY-MM-DD`, when known.
    pub installed_on: Option<String>,
    pub source: Source,
    /// The program's own uninstaller, as one command line (Windows) or argv.
    pub uninstall: Option<Command>,
    /// The same, unattended, when the program offers it.
    pub quiet_uninstall: Option<Command>,
    /// The winget package id, when winget knows this app (Windows).
    pub winget_id: Option<String>,
    /// Removing it needs admin rights (HKLM, /Applications owned by root, a
    /// system package…).
    pub needs_admin: bool,
    /// Words the leftover search should also look for: bundle id, Flatpak
    /// id, package name, the install folder's name…
    pub hints: Vec<String>,
}

/// A program to run. Windows uninstall strings are command lines that do not
/// split cleanly into argv, so they stay one string there.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Command {
    Line { line: String },
    Argv { argv: Vec<String> },
}

/// How hard the leftover search looks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ScanLevel {
    /// Only places named exactly after the app, its publisher or its folder.
    Safe,
    /// Also names that merely contain a search word, and system places.
    Deep,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Confidence {
    /// Pre-selected: the name matches, or a trace recorded it.
    High,
    /// Shown, not selected: only a part of the name matches.
    Medium,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ItemKind {
    File,
    Dir,
    /// A registry key with everything below it (Windows).
    RegKey,
    /// One registry value (Windows).
    RegValue,
}

/// Something found after an uninstall that probably belonged to the app.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Leftover {
    pub kind: ItemKind,
    /// A file system path, or `HKCU\Software\Vendor\App` for the registry;
    /// for a value `HKCU\Software\…\Run` plus [`Leftover::value`].
    pub path: String,
    pub value: Option<String>,
    /// Bytes on disk, for files and folders.
    pub size: Option<u64>,
    pub confidence: Confidence,
    /// Which search word matched, or `trace` — shown next to the item.
    pub reason: String,
    pub needs_admin: bool,
}

/// Where an autostart entry lives.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AutostartPlace {
    /// Windows `…\CurrentVersion\Run` in HKCU.
    RunUser,
    /// Windows `…\CurrentVersion\Run` in HKLM (also the 32-bit view).
    RunMachine,
    /// Windows Startup folder of the user.
    StartupUser,
    /// Windows Startup folder for all users.
    StartupCommon,
    /// macOS `~/Library/LaunchAgents`.
    LaunchAgentUser,
    /// macOS `/Library/LaunchAgents`.
    LaunchAgentSystem,
    /// macOS `/Library/LaunchDaemons`.
    LaunchDaemon,
    /// Linux `~/.config/autostart` or `/etc/xdg/autostart`.
    Xdg,
    /// Linux `systemctl --user` unit.
    SystemdUser,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AutostartEntry {
    /// Stable id: place plus path/name, so the page can toggle it.
    pub id: String,
    pub name: String,
    pub command: String,
    pub place: AutostartPlace,
    /// Path or registry location, shown in the details.
    pub location: String,
    pub enabled: bool,
    pub needs_admin: bool,
}

/// One thing that was moved into the quarantine.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuarantineItem {
    pub kind: ItemKind,
    /// Where it was, as in [`Leftover::path`].
    pub original: String,
    pub value: Option<String>,
    /// Its name inside the batch folder (`files/0`, `registry/3.json`…).
    pub stored: String,
    pub size: Option<u64>,
}

/// One "sweep": everything removed for one app at one time.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuarantineBatch {
    pub id: String,
    pub app_name: String,
    /// Seconds since the Unix epoch.
    pub created: u64,
    pub items: Vec<QuarantineItem>,
    /// The batch was made by the elevated helper; restoring it needs it too.
    pub needs_admin: bool,
}

/// What an install trace recorded: everything that was new afterwards.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Trace {
    pub id: String,
    /// The name the user gave, or the app the install added.
    pub name: String,
    pub created: u64,
    /// App ids that appeared in the app list during the trace.
    pub app_ids: Vec<String>,
    pub items: Vec<Leftover>,
}
