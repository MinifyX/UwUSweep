//! Installing, updating and removing UwUSweep for the current Windows user.
//!
//! Everything lives under the user's profile: the app in
//! `%LOCALAPPDATA%\Programs\UwUSweep`, shortcuts in the Start menu
//! (and on the desktop if wanted), and registry entries under
//! `HKEY_CURRENT_USER`. No administrator rights are needed: the app asks for
//! them itself (UAC) only when an uninstall or a cleanup needs them.
//!
//! `UWUSWEEP_SETUP_SANDBOX=<folder>` redirects all of it (files, shortcuts,
//! registry under `HKCU\Software\UwUSweep-Setup-Sandbox`) for testing.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use winreg::enums::{HKEY_CURRENT_USER, KEY_READ};
use winreg::RegKey;

use crate::system;

pub const APP_EXE: &str = "UwUSweep.exe";
pub const UNINSTALL_EXE: &str = "uninstall.exe";
pub const APP_ID: &str = "app.uwusweep.desktop";
const SHORTCUT: &str = "UwUSweep.lnk";
const HOMEPAGE: &str = "https://github.com/MinifyX/UwUSweep";
const UNINSTALL_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Uninstall\UwUSweep";
const SETUP_KEY: &str = r"Software\UwUSweep\Setup";

static PAYLOAD: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/payload.zst"));
const PAYLOAD_SIZE: &str = env!("UWUSWEEP_SETUP_PAYLOAD_SIZE");
pub fn has_payload() -> bool {
    !PAYLOAD.is_empty()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Options {
    pub dir: String,
    pub desktop_shortcut: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Installed {
    pub dir: String,
    pub version: Option<String>,
}

/// Where things go on this machine.
pub struct Layout {
    pub default_dir: PathBuf,
    pub start_menu: PathBuf,
    pub desktop: PathBuf,
    /// The settings, the quarantine and the install traces.
    pub roaming_data: PathBuf,
    /// WebView2's own data, and whatever the app keeps that needn't roam.
    pub local_data: PathBuf,
    registry_prefix: String,
    pub sandbox: bool,
}

impl Layout {
    pub fn detect() -> Self {
        match std::env::var_os("UWUSWEEP_SETUP_SANDBOX").filter(|dir| !dir.is_empty()) {
            Some(dir) => Self::sandbox(Path::new(&dir)),
            None => {
                let folders = system::folders();
                Self {
                    default_dir: folders.user_programs.join("UwUSweep"),
                    start_menu: folders.start_menu,
                    desktop: folders.desktop,
                    roaming_data: folders.roaming.join(APP_ID),
                    local_data: folders.local.join(APP_ID),
                    registry_prefix: String::new(),
                    sandbox: false,
                }
            }
        }
    }

    pub fn sandbox(root: &Path) -> Self {
        Self {
            default_dir: root.join(r"Programs\UwUSweep"),
            start_menu: root.join("StartMenu"),
            desktop: root.join("Desktop"),
            roaming_data: root.join("Roaming").join(APP_ID),
            local_data: root.join("Local").join(APP_ID),
            registry_prefix: format!(
                r"Software\UwUSweep-Setup-Sandbox\{}\",
                root.file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default()
            ),
            sandbox: true,
        }
    }

    fn key(&self, path: &str) -> String {
        format!("{}{path}", self.registry_prefix)
    }

    fn open(&self, path: &str) -> Option<RegKey> {
        RegKey::predef(HKEY_CURRENT_USER)
            .open_subkey_with_flags(self.key(path), KEY_READ)
            .ok()
    }

    fn create(&self, path: &str) -> Result<RegKey, String> {
        RegKey::predef(HKEY_CURRENT_USER)
            .create_subkey(self.key(path))
            .map(|(key, _)| key)
            .map_err(|e| format!("Couldn't write to the registry ({path}): {e}"))
    }

    fn remove_tree(&self, path: &str) {
        let _ = RegKey::predef(HKEY_CURRENT_USER).delete_subkey_all(self.key(path));
    }

    /// What is installed right now, if anything.
    pub fn installed(&self) -> Option<Installed> {
        let key = self.open(SETUP_KEY)?;
        let dir = key.get_value::<String, _>("InstallDir").ok()?;
        Path::new(&dir).join(APP_EXE).exists().then(|| Installed {
            dir,
            version: key.get_value("Version").ok(),
        })
    }

    /// Options from the last install, or the defaults.
    pub fn remembered_options(&self) -> Options {
        let key = self.open(SETUP_KEY);
        let flag = |name: &str, default: bool| {
            key.as_ref()
                .and_then(|k| k.get_value::<u32, _>(name).ok())
                .map_or(default, |v| v != 0)
        };
        let dir = self.installed().map_or_else(
            || self.default_dir.display().to_string(),
            |installed| installed.dir,
        );
        Options {
            dir,
            desktop_shortcut: flag("DesktopShortcut", true),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Step {
    Prepare,
    Copy,
    Shortcuts,
    Register,
    Cleanup,
    Done,
}

pub type Progress<'a> = &'a mut dyn FnMut(Step, f64);

/// Writes a file next to its destination first, then swaps it in. A running
/// program keeps its file locked for a moment after it ends, hence the retries.
fn replace_file(from: &Path, to: &Path) -> Result<(), String> {
    let mut attempt = 0;
    loop {
        match std::fs::rename(from, to) {
            Ok(()) => return Ok(()),
            Err(_) if attempt < 19 => std::thread::sleep(Duration::from_millis(250)),
            Err(e) => return Err(format!("Couldn't replace {}: {e}", to.display())),
        }
        attempt += 1;
    }
}

/// Unpack one packed file to `target`, telling `progress` how far it got (0 to 1).
fn extract(
    payload: &[u8],
    size: &str,
    target: &Path,
    progress: &mut dyn FnMut(f64),
) -> Result<(), String> {
    if payload.is_empty() {
        return Err("This setup was built without UwUSweep inside (a development build).".into());
    }
    let total: u64 = size.parse().unwrap_or(1).max(1);
    let mut decoder =
        zstd::Decoder::new(payload).map_err(|e| format!("The packed app is damaged: {e}"))?;
    let mut file = std::fs::File::create(target)
        .map_err(|e| format!("Couldn't write {}: {e}", target.display()))?;
    let mut buffer = vec![0u8; 256 * 1024];
    let mut written = 0u64;
    loop {
        let read = decoder
            .read(&mut buffer)
            .map_err(|e| format!("The packed app is damaged: {e}"))?;
        if read == 0 {
            break;
        }
        std::io::Write::write_all(&mut file, &buffer[..read])
            .map_err(|e| format!("Couldn't write {}: {e}", target.display()))?;
        written += read as u64;
        progress(written as f64 / total as f64);
    }
    file.sync_all()
        .map_err(|e| format!("Couldn't write {}: {e}", target.display()))?;
    if written != total {
        return Err("The packed app is incomplete.".into());
    }
    Ok(())
}

/// The packed program into `dir` as `<name><suffix>`, reporting through the
/// copy step.
fn extract_all(dir: &Path, suffix: &str, progress: Progress) -> Result<(), String> {
    extract(
        PAYLOAD,
        PAYLOAD_SIZE,
        &dir.join(format!("{APP_EXE}{suffix}")),
        &mut |done| progress(Step::Copy, done),
    )
}

/// Unpacks the program into `dir` (which must not exist yet) and checks that
/// it looks like one — `--check-payload`, for CI.
pub fn check_payload(dir: &Path) -> Result<String, String> {
    if dir.exists() {
        return Err(format!("{} exists already.", dir.display()));
    }
    std::fs::create_dir_all(dir).map_err(|e| format!("Couldn't create {}: {e}", dir.display()))?;
    extract_all(dir, "", &mut |_, _| {})?;
    let path = dir.join(APP_EXE);
    let bytes = std::fs::read(&path).map_err(|e| format!("{} is missing: {e}", path.display()))?;
    if !bytes.starts_with(b"MZ") {
        return Err(format!("{} isn't what it should be.", path.display()));
    }
    Ok(format!("ok  {} ({} bytes)", path.display(), bytes.len()))
}

fn quoted(path: &Path) -> String {
    format!("\"{}\"", path.display())
}

fn dir_size_kb(dir: &Path) -> u32 {
    let bytes: u64 = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| entry.metadata().ok())
        .filter(|meta| meta.is_file())
        .map(|meta| meta.len())
        .sum();
    (bytes / 1024).min(u32::MAX as u64) as u32
}

/// Stops UwUSweep if it runs from `dir`. Skipped in the sandbox, which must
/// never touch real processes.
pub fn stop_app(layout: &Layout, dir: &Path) -> Result<(), String> {
    if layout.sandbox {
        return Ok(());
    }
    system::stop_processes(&dir.join(APP_EXE))
}

/// UwUSweep is open. Closing it could cut a running cleanup short, so the
/// setup asks first.
pub fn app_running(layout: &Layout, dir: &Path) -> bool {
    !layout.sandbox && !system::processes_of(&dir.join(APP_EXE)).is_empty()
}

/// The folder the setup installs into for a folder the person picked: a
/// "UwUSweep" folder inside it, unless it is one already.
pub fn folder_for(mut picked: PathBuf) -> PathBuf {
    if !picked
        .file_name()
        .is_some_and(|name| name.eq_ignore_ascii_case("UwUSweep"))
    {
        picked.push("UwUSweep");
    }
    picked
}

/// Starts the installed UwUSweep. Nothing in the sandbox.
pub fn launch(layout: &Layout, dir: &Path) -> Result<(), String> {
    if layout.sandbox {
        return Ok(());
    }
    system::spawn_detached(&dir.join(APP_EXE), &[])
}

pub fn install(
    layout: &Layout,
    options: &Options,
    version: &str,
    progress: Progress,
) -> Result<(), String> {
    let dir = PathBuf::from(options.dir.trim());
    if !dir.is_absolute() {
        return Err("Please pick a full folder path.".into());
    }
    progress(Step::Prepare, 0.0);
    std::fs::create_dir_all(&dir).map_err(|e| format!("Couldn't create {}: {e}", dir.display()))?;
    stop_app(layout, &dir)?;
    progress(Step::Prepare, 1.0);

    extract_all(&dir, ".new", progress)?;
    replace_file(&dir.join(format!("{APP_EXE}.new")), &dir.join(APP_EXE))?;
    let app = dir.join(APP_EXE);

    let uninstaller = dir.join(UNINSTALL_EXE);
    let me =
        std::env::current_exe().map_err(|e| format!("Couldn't find the setup program: {e}"))?;
    if !me.eq(&uninstaller) {
        let copy = dir.join("uninstall.exe.new");
        std::fs::copy(&me, &copy).map_err(|e| format!("Couldn't write {}: {e}", copy.display()))?;
        replace_file(&copy, &uninstaller)?;
    }

    progress(Step::Shortcuts, 0.0);
    let shortcut = system::Shortcut {
        target: &app,
        arguments: "",
        description: "UwUSweep",
        app_id: APP_ID,
    };
    system::create_shortcut(&layout.start_menu.join(SHORTCUT), &shortcut)?;
    let on_desktop = layout.desktop.join(SHORTCUT);
    if options.desktop_shortcut {
        system::create_shortcut(&on_desktop, &shortcut)?;
    } else {
        let _ = std::fs::remove_file(&on_desktop);
    }
    progress(Step::Shortcuts, 1.0);

    progress(Step::Register, 0.0);
    register(layout, &dir, options, version)?;
    progress(Step::Register, 1.0);
    progress(Step::Done, 1.0);
    Ok(())
}

fn register(layout: &Layout, dir: &Path, options: &Options, version: &str) -> Result<(), String> {
    let app = dir.join(APP_EXE);
    let uninstaller = dir.join(UNINSTALL_EXE);
    let write = |key: &RegKey, name: &str, value: &str| {
        key.set_value(name, &value)
            .map_err(|e| format!("Couldn't write to the registry ({name}): {e}"))
    };
    let write_dword = |key: &RegKey, name: &str, value: u32| {
        key.set_value(name, &value)
            .map_err(|e| format!("Couldn't write to the registry ({name}): {e}"))
    };

    // An older setup may have written values this one doesn't; start clean.
    layout.remove_tree(UNINSTALL_KEY);
    let entry = layout.create(UNINSTALL_KEY)?;
    write(&entry, "DisplayName", "UwUSweep")?;
    write(&entry, "DisplayVersion", version)?;
    write(&entry, "Publisher", "UwUSweep")?;
    write(&entry, "DisplayIcon", &format!("{},0", app.display()))?;
    write(&entry, "InstallLocation", &dir.display().to_string())?;
    write(
        &entry,
        "UninstallString",
        &format!("{} --uninstall", quoted(&uninstaller)),
    )?;
    write(&entry, "URLInfoAbout", HOMEPAGE)?;
    write(&entry, "HelpLink", HOMEPAGE)?;
    write_dword(&entry, "EstimatedSize", dir_size_kb(dir))?;
    write_dword(&entry, "NoModify", 1)?;
    write_dword(&entry, "NoRepair", 1)?;

    let setup = layout.create(SETUP_KEY)?;
    write(&setup, "InstallDir", &dir.display().to_string())?;
    write(&setup, "Version", version)?;
    write_dword(&setup, "DesktopShortcut", options.desktop_shortcut.into())?;
    Ok(())
}

pub fn uninstall(
    layout: &Layout,
    dir: &Path,
    keep_data: bool,
    progress: Progress,
) -> Result<(), String> {
    progress(Step::Prepare, 0.0);
    stop_app(layout, dir)?;
    progress(Step::Prepare, 1.0);

    progress(Step::Shortcuts, 0.0);
    let _ = std::fs::remove_file(layout.start_menu.join(SHORTCUT));
    let _ = std::fs::remove_file(layout.desktop.join(SHORTCUT));
    progress(Step::Shortcuts, 1.0);

    progress(Step::Register, 0.0);
    layout.remove_tree(UNINSTALL_KEY);
    layout.remove_tree(r"Software\UwUSweep");
    progress(Step::Register, 1.0);

    progress(Step::Copy, 0.0);
    for file in [
        APP_EXE,
        UNINSTALL_EXE,
        "UwUSweep.exe.new",
        "uninstall.exe.new",
    ] {
        let path = dir.join(file);
        if path.exists() && std::fs::remove_file(&path).is_err() && file == APP_EXE {
            return Err(format!(
                "Couldn't remove {}. Is UwUSweep still open?",
                path.display()
            ));
        }
    }
    let _ = std::fs::remove_dir(dir);
    progress(Step::Copy, 1.0);

    if !keep_data {
        progress(Step::Cleanup, 0.0);
        for folder in [&layout.roaming_data, &layout.local_data] {
            if folder.exists() {
                std::fs::remove_dir_all(folder)
                    .map_err(|e| format!("Couldn't delete {}: {e}", folder.display()))?;
            }
        }
        progress(Step::Cleanup, 1.0);
    }
    progress(Step::Done, 1.0);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Sandbox {
        _dir: tempfile::TempDir,
        layout: Layout,
    }

    impl Sandbox {
        fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let layout = Layout::sandbox(dir.path());
            Self { _dir: dir, layout }
        }
    }

    impl Drop for Sandbox {
        fn drop(&mut self) {
            let key = self
                .layout
                .registry_prefix
                .trim_end_matches('\\')
                .to_string();
            let _ = RegKey::predef(HKEY_CURRENT_USER).delete_subkey_all(key);
        }
    }

    #[test]
    fn remembers_defaults_until_something_is_installed() {
        let sandbox = Sandbox::new();
        assert!(sandbox.layout.installed().is_none());
        let options = sandbox.layout.remembered_options();
        assert!(options.dir.ends_with(r"Programs\UwUSweep"));
        assert!(options.desktop_shortcut);
    }

    #[test]
    fn registers_and_unregisters_everything() {
        let sandbox = Sandbox::new();
        let layout = &sandbox.layout;
        let dir = layout.default_dir.clone();
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(APP_EXE), b"app").unwrap();
        std::fs::write(dir.join(UNINSTALL_EXE), b"setup").unwrap();
        let options = Options {
            dir: dir.display().to_string(),
            desktop_shortcut: false,
        };
        register(layout, &dir, &options, "0.1.0").unwrap();

        let installed = layout.installed().unwrap();
        assert_eq!(installed.version.as_deref(), Some("0.1.0"));
        assert!(!layout.remembered_options().desktop_shortcut);
        let command: String = layout
            .open(UNINSTALL_KEY)
            .unwrap()
            .get_value("UninstallString")
            .unwrap();
        assert!(command.ends_with("\" --uninstall"));

        std::fs::create_dir_all(&layout.roaming_data).unwrap();
        std::fs::write(layout.roaming_data.join("settings.json"), b"{}").unwrap();
        uninstall(layout, &dir, true, &mut |_, _| {}).unwrap();
        assert!(!dir.exists(), "the program is gone");
        assert!(
            layout.roaming_data.join("settings.json").exists(),
            "settings are kept"
        );
        assert!(layout.open(UNINSTALL_KEY).is_none());
        assert!(layout.open(SETUP_KEY).is_none());

        std::fs::create_dir_all(&dir).unwrap();
        uninstall(layout, &dir, false, &mut |_, _| {}).unwrap();
        assert!(
            !layout.roaming_data.exists(),
            "settings are deleted on request"
        );
    }

    #[test]
    fn a_folder_the_person_picked_gets_a_folder_of_ours() {
        assert_eq!(
            folder_for(PathBuf::from(r"D:\Apps")),
            PathBuf::from(r"D:\Apps\UwUSweep")
        );
        assert_eq!(
            folder_for(PathBuf::from(r"D:\uwusweep")),
            PathBuf::from(r"D:\uwusweep")
        );
    }
}
