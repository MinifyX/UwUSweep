//! UwUSweep's shell: hands the page the core's app list, leftover search,
//! quarantine, install traces and autostart list as commands, and starts the
//! elevated helper (this same binary) when a step needs admin rights.

mod log;

use std::path::{Path, PathBuf};

use parking_lot::Mutex;
use serde::Serialize;
use tauri::{Manager, State};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_opener::OpenerExt;
use uwusweep_core::helper::{self, Op, Outcome};
use uwusweep_core::quarantine::{real_path, valid_id, PutReport, Quarantine, ADMIN_PREFIX};
use uwusweep_core::snapshot::{self, Snapshot, Traces};
use uwusweep_core::{
    platform, scan, App, AutostartEntry, ItemKind, Leftover, QuarantineBatch, QuarantineItem,
    ScanLevel, Source, Trace,
};

/// What a picked installer may be; anything else is not started.
const INSTALLER_EXTENSIONS: &[&str] = &[
    "exe",
    "msi",
    "msix",
    "msixbundle",
    "appx",
    "dmg",
    "pkg",
    "deb",
    "rpm",
    "AppImage",
];

struct AppState {
    data: PathBuf,
    elevated: bool,
    /// The last app list, so the page can name an app by its id.
    apps: Mutex<Vec<App>>,
    /// The last leftover search. Only these items may go into the quarantine,
    /// with the flags the search gave them, whatever the page sends.
    scanned: Mutex<Vec<Leftover>>,
    /// The snapshot of a running install trace.
    trace: Mutex<Option<Snapshot>>,
    /// The installer the user picked last; the only one `installer_open`
    /// starts.
    picked: Mutex<Option<PathBuf>>,
}

impl AppState {
    fn quarantine_root(&self) -> PathBuf {
        self.data.join("quarantine")
    }

    fn quarantine(&self) -> Quarantine {
        Quarantine::new(self.quarantine_root())
    }

    /// The user's own quarantine for a restore or purge. Elevated, only
    /// where it really is: a link to it could lead the admin elsewhere.
    fn own_quarantine(&self) -> Result<Quarantine> {
        let root = self.quarantine_root();
        if self.elevated && root.exists() {
            let real = std::fs::canonicalize(&root).map_err(text)?;
            let real = real.to_string_lossy();
            let real = real.strip_prefix(r"\\?\").unwrap_or(&real);
            if !real.eq_ignore_ascii_case(&root.to_string_lossy()) {
                return Err("the quarantine folder is a link".into());
            }
        }
        Ok(Quarantine::new(root))
    }

    fn traces(&self) -> Traces {
        Traces::new(self.data.join("traces"))
    }

    fn app(&self, id: &str) -> Result<App> {
        self.apps
            .lock()
            .iter()
            .find(|app| app.id == id)
            .cloned()
            .ok_or_else(|| "unknown app".to_owned())
    }

    /// Runs `op` in the elevated helper, or right here when this process
    /// already has admin rights; `Err("cancelled")` when the user declined
    /// the admin prompt.
    fn as_admin(&self, op: Op) -> Result<(Option<PutReport>, Failed)> {
        let outcome = if self.elevated {
            Some(helper::run_op(&op))
        } else {
            let exe = std::env::current_exe().map_err(text)?;
            let jobs = self.data.join("jobs");
            std::fs::create_dir_all(&jobs).map_err(text)?;
            helper::run_as_admin(&exe, &[op], &jobs)
                .map_err(text)?
                .into_iter()
                .next()
        };
        match outcome {
            Some(Outcome::Ok { report, failed }) => Ok((report, failed)),
            Some(Outcome::Err { message }) => Err(message),
            None => Err("the helper gave no result".into()),
        }
    }
}

type Result<T> = std::result::Result<T, String>;
/// Path and why it could not be handled.
type Failed = Vec<(String, String)>;

fn text(error: impl std::fmt::Display) -> String {
    error.to_string()
}

/// Runs `work` off the async runtime: scans and uninstallers take a while.
async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T> + Send + 'static,
) -> Result<T> {
    tauri::async_runtime::spawn_blocking(work)
        .await
        .map_err(text)?
}

fn state(app: &tauri::AppHandle) -> State<'_, AppState> {
    app.state::<AppState>()
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PlatformInfo {
    os: &'static str,
    elevated: bool,
    restore_point_supported: bool,
    version: &'static str,
}

#[tauri::command]
fn platform_info(state: State<'_, AppState>) -> PlatformInfo {
    PlatformInfo {
        os: if cfg!(windows) {
            "windows"
        } else if cfg!(target_os = "macos") {
            "macos"
        } else {
            "linux"
        },
        elevated: state.elevated,
        restore_point_supported: cfg!(windows),
        version: env!("CARGO_PKG_VERSION"),
    }
}

#[tauri::command]
async fn apps_list(app: tauri::AppHandle) -> Result<Vec<App>> {
    blocking(move || {
        let apps = platform::list_apps().map_err(text)?;
        *state(&app).apps.lock() = apps.clone();
        Ok(apps)
    })
    .await
}

#[tauri::command]
async fn leftovers_scan(
    app: tauri::AppHandle,
    app_id: String,
    level: ScanLevel,
) -> Result<Vec<Leftover>> {
    blocking(move || {
        let state = state(&app);
        let target = state.app(&app_id)?;
        let traces = state.traces().list().unwrap_or_else(|error| {
            tracing::warn!(%error, "traces");
            Vec::new()
        });
        let found = scan::find_leftovers(&target, level, &traces);
        *state.scanned.lock() = found.clone();
        Ok(found)
    })
    .await
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct UninstallReport {
    /// The restore point failed; the uninstall went ahead anyway.
    restore_point_error: Option<String>,
}

/// System packages need root; everything else either runs as the user or
/// asks for admin itself (Windows uninstallers bring their own UAC prompt).
fn uninstall_needs_helper(app: &App, elevated: bool) -> bool {
    app.needs_admin
        && !elevated
        && matches!(
            app.source,
            Source::Package { .. } | Source::Snap { .. } | Source::Flatpak { user: false, .. }
        )
}

#[tauri::command]
async fn uninstall_start(
    app: tauri::AppHandle,
    app_id: String,
    quiet: bool,
    restore_point: bool,
) -> Result<UninstallReport> {
    blocking(move || {
        let state = state(&app);
        let target = state.app(&app_id)?;
        let mut report = UninstallReport {
            restore_point_error: None,
        };
        if restore_point && cfg!(windows) {
            let description = format!("UwUSweep: {}", target.name);
            let made = if state.elevated {
                platform::create_restore_point(&description).map_err(text)
            } else {
                state.as_admin(Op::RestorePoint { description }).map(drop)
            };
            if let Err(error) = made {
                tracing::warn!(%error, "restore point");
                report.restore_point_error = Some(error);
            }
        }
        if uninstall_needs_helper(&target, state.elevated) {
            state.as_admin(Op::Uninstall { app: target, quiet })?;
        } else {
            platform::uninstall(&target, quiet).map_err(text)?;
        }
        Ok(report)
    })
    .await
}

/// What moving leftovers aside did: up to two batches (the user's own items,
/// and those the helper moved), and what could not be moved.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SweepReport {
    batches: Vec<QuarantineBatch>,
    failed: Vec<(String, String)>,
}

fn same_item(a: &Leftover, b: &Leftover) -> bool {
    a.kind == b.kind && a.path == b.path && a.value == b.value
}

#[tauri::command]
async fn quarantine_put(
    app: tauri::AppHandle,
    app_name: String,
    items: Vec<Leftover>,
) -> Result<SweepReport> {
    blocking(move || {
        let state = state(&app);
        let mut failed = Vec::new();
        let (mut own, mut admin) = (Vec::new(), Vec::new());
        {
            let scanned = state.scanned.lock();
            for item in &items {
                match scanned.iter().find(|known| same_item(known, item)) {
                    Some(known) if scan::is_protected_item(known) => {
                        failed.push((known.path.clone(), "protected".into()))
                    }
                    // Elevated, everything goes to the admins' quarantine:
                    // the user's own is theirs to change.
                    Some(known) if known.needs_admin || state.elevated => admin.push(known.clone()),
                    Some(known) => own.push(known.clone()),
                    None => failed.push((item.path.clone(), "not from the last search".into())),
                }
            }
        }
        let mut batches = Vec::new();
        if !own.is_empty() {
            let report = state.quarantine().put(&app_name, &own).map_err(text)?;
            batches.extend(report.batch);
            failed.extend(report.failed);
        }
        if !admin.is_empty() {
            let op = Op::Quarantine {
                app_name,
                items: admin.clone(),
            };
            match state.as_admin(op) {
                Ok((report, more)) => {
                    if let Some(report) = report {
                        batches.extend(report.batch);
                        failed.extend(report.failed);
                    }
                    failed.extend(more);
                }
                Err(error) => {
                    failed.extend(admin.into_iter().map(|item| (item.path, error.clone())))
                }
            }
        }
        // Moved items are gone; a second click must not find them again.
        state
            .scanned
            .lock()
            .retain(|item| failed.iter().any(|(path, _)| *path == item.path));
        Ok(SweepReport { batches, failed })
    })
    .await
}

fn is_admin_batch(id: &str) -> Result<bool> {
    if !valid_id(id) {
        return Err("bad batch id".into());
    }
    Ok(id.starts_with(ADMIN_PREFIX))
}

/// Both quarantines: the user's own and the elevated helper's, which users
/// may read but not change.
#[tauri::command]
async fn quarantine_list(app: tauri::AppHandle) -> Result<Vec<QuarantineBatch>> {
    blocking(move || {
        let mut batches = state(&app).quarantine().list().map_err(text)?;
        batches.retain(|batch| !batch.id.starts_with(ADMIN_PREFIX));
        let admin = Quarantine::new(platform::admin_quarantine_root())
            .list()
            .unwrap_or_else(|error| {
                tracing::warn!(%error, "admin quarantine");
                Vec::new()
            });
        batches.extend(admin.into_iter().map(|batch| QuarantineBatch {
            needs_admin: true,
            ..batch
        }));
        batches.sort_by(|a, b| (b.created, &b.id).cmp(&(a.created, &a.id)));
        Ok(batches)
    })
    .await
}

/// Elevated, a restore from the user's own quarantine only goes back into
/// their home and `HKCU`: its manifest is theirs to change.
fn own_place(item: &QuarantineItem, home: Option<&Path>) -> bool {
    match item.kind {
        ItemKind::File | ItemKind::Dir => home.is_some_and(|home| {
            real_path(Path::new(&item.original)).is_some_and(|real| {
                scan::is_under(&real.to_string_lossy(), &home.to_string_lossy())
            })
        }),
        _ => item
            .original
            .get(..5)
            .is_some_and(|hive| hive.eq_ignore_ascii_case(r"HKCU\")),
    }
}

#[tauri::command]
async fn quarantine_restore(app: tauri::AppHandle, id: String) -> Result<Vec<(String, String)>> {
    blocking(move || {
        let state = state(&app);
        if is_admin_batch(&id)? {
            state.as_admin(Op::Restore { id }).map(|(_, failed)| failed)
        } else if state.elevated {
            let home = scan::home_dir().and_then(|home| real_path(&home));
            state
                .own_quarantine()?
                .restore_where(&id, |item| own_place(item, home.as_deref()))
                .map_err(text)
        } else {
            state.quarantine().restore(&id).map_err(text)
        }
    })
    .await
}

#[tauri::command]
async fn quarantine_purge(app: tauri::AppHandle, id: String) -> Result<()> {
    blocking(move || {
        let state = state(&app);
        if is_admin_batch(&id)? {
            state.as_admin(Op::Purge { id }).map(drop)
        } else {
            state.own_quarantine()?.purge(&id).map_err(text)
        }
    })
    .await
}

/// Called once on start with the kept days from the settings. The helper's
/// batches only expire here when the app runs with admin rights; otherwise
/// the user purges them by hand.
#[tauri::command]
async fn quarantine_cleanup(app: tauri::AppHandle, days: u64) -> Result<usize> {
    blocking(move || {
        let state = state(&app);
        let mut n = state
            .own_quarantine()?
            .purge_older_than(days.max(1))
            .map_err(text)?;
        if state.elevated {
            n += Quarantine::admin()
                .and_then(|q| q.purge_older_than(days.max(1)))
                .map_err(text)?;
        }
        Ok(n)
    })
    .await
}

/// Shows a batch's folder in the file manager.
#[tauri::command]
fn quarantine_reveal(app: tauri::AppHandle, id: String) -> Result<()> {
    let root = if is_admin_batch(&id)? {
        platform::admin_quarantine_root()
    } else {
        state(&app).quarantine_root()
    };
    let dir = root.join(id);
    app.opener().reveal_item_in_dir(dir).map_err(text)
}

#[tauri::command]
async fn autostart_list() -> Result<Vec<AutostartEntry>> {
    blocking(|| platform::autostart_list().map_err(text)).await
}

#[tauri::command]
async fn autostart_set(app: tauri::AppHandle, entry: AutostartEntry, enabled: bool) -> Result<()> {
    blocking(move || {
        let state = state(&app);
        // Trust the system's entry, not the page's copy of it.
        let entry = platform::autostart_list()
            .map_err(text)?
            .into_iter()
            .find(|known| known.id == entry.id)
            .ok_or_else(|| "unknown autostart entry".to_owned())?;
        if entry.needs_admin && !state.elevated {
            state.as_admin(Op::Autostart { entry, enabled }).map(drop)
        } else {
            platform::autostart_set(&entry, enabled).map_err(text)
        }
    })
    .await
}

#[tauri::command]
async fn trace_start(app: tauri::AppHandle) -> Result<()> {
    blocking(move || {
        let before = snapshot::take().map_err(text)?;
        *state(&app).trace.lock() = Some(before);
        Ok(())
    })
    .await
}

#[tauri::command]
async fn trace_finish(app: tauri::AppHandle, name: String) -> Result<Trace> {
    blocking(move || {
        let state = state(&app);
        let before = state
            .trace
            .lock()
            .take()
            .ok_or_else(|| "no trace running".to_owned())?;
        let after = snapshot::take().map_err(text)?;
        let (items, app_ids) = snapshot::diff(&before, &after);
        let name = name.trim();
        let name = if !name.is_empty() {
            name.chars().take(120).collect()
        } else {
            app_ids
                .iter()
                .find_map(|id| after.apps.get(id).cloned())
                .unwrap_or_else(|| "Installation".into())
        };
        let created = uwusweep_core::now();
        let trace = Trace {
            id: format!("{created}-{}", std::process::id()),
            name,
            created,
            app_ids,
            items,
        };
        state.traces().save(&trace).map_err(text)?;
        Ok(trace)
    })
    .await
}

#[tauri::command]
fn trace_cancel(state: State<'_, AppState>) {
    state.trace.lock().take();
}

#[tauri::command]
async fn traces_list(app: tauri::AppHandle) -> Result<Vec<Trace>> {
    blocking(move || state(&app).traces().list().map_err(text)).await
}

#[tauri::command]
async fn trace_delete(app: tauri::AppHandle, id: String) -> Result<()> {
    blocking(move || state(&app).traces().delete(&id).map_err(text)).await
}

/// Lets the user pick an installer to start during a trace.
#[tauri::command]
async fn install_pick(app: tauri::AppHandle) -> Result<Option<String>> {
    blocking(move || {
        let picked = app
            .dialog()
            .file()
            .add_filter("Installer", INSTALLER_EXTENSIONS)
            .blocking_pick_file()
            .and_then(|file| file.into_path().ok());
        *state(&app).picked.lock() = picked.clone();
        Ok(picked.map(|path| path.to_string_lossy().into_owned()))
    })
    .await
}

fn is_installer(path: &Path) -> bool {
    path.is_file()
        && path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|ext| {
                INSTALLER_EXTENSIONS
                    .iter()
                    .any(|known| known.eq_ignore_ascii_case(ext))
            })
}

/// Starts the picked installer the way the system would on a double-click.
#[tauri::command]
fn installer_open(app: tauri::AppHandle, path: String) -> Result<()> {
    // Only what the user picked in the dialog, not any path the page names.
    let picked = state(&app).picked.lock().take();
    if picked.as_deref() != Some(Path::new(&path)) || !is_installer(Path::new(&path)) {
        return Err("not an installer".into());
    }
    app.opener().open_path(path, None::<&str>).map_err(text)
}

/// Shows a leftover in the file manager. Reveal only: nothing is opened or run.
#[tauri::command]
fn reveal_path(app: tauri::AppHandle, path: String) -> Result<()> {
    let path = PathBuf::from(path);
    if !path.is_absolute() || !path.exists() {
        return Err("no such path".into());
    }
    app.opener().reveal_item_in_dir(path).map_err(text)
}

/// Opens a web page in the browser: only https, only what the page links to.
#[tauri::command]
fn open_link(app: tauri::AppHandle, url: String) -> Result<()> {
    if !url.starts_with("https://") {
        return Err("only https links".into());
    }
    app.opener().open_url(url, None::<&str>).map_err(text)
}

/// The detailed log on or off (Settings → General).
#[tauri::command]
fn log_detail(on: bool) {
    log::set_detailed(on);
}

/// Shows the log folder in the file manager, for attaching the log to an issue.
#[tauri::command]
fn open_log_folder(app: tauri::AppHandle) -> Result<()> {
    let dir = app.path().app_log_dir().map_err(text)?;
    std::fs::create_dir_all(&dir).map_err(text)?;
    app.opener()
        .open_path(dir.to_string_lossy(), None::<&str>)
        .map_err(text)
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            log::init(&app.path().app_log_dir()?);
            app.manage(AppState {
                data: app.path().app_data_dir()?,
                elevated: platform::is_elevated(),
                apps: Mutex::new(Vec::new()),
                scanned: Mutex::new(Vec::new()),
                trace: Mutex::new(None),
                picked: Mutex::new(None),
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            platform_info,
            apps_list,
            leftovers_scan,
            uninstall_start,
            quarantine_put,
            quarantine_list,
            quarantine_restore,
            quarantine_purge,
            quarantine_cleanup,
            quarantine_reveal,
            autostart_list,
            autostart_set,
            trace_start,
            trace_finish,
            trace_cancel,
            traces_list,
            trace_delete,
            install_pick,
            installer_open,
            reveal_path,
            open_link,
            log_detail,
            open_log_folder,
        ])
        .run(tauri::generate_context!())
        .expect("UwUSweep failed to start");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn leftover(path: &str, kind: ItemKind) -> Leftover {
        Leftover {
            kind,
            path: path.into(),
            value: None,
            size: None,
            confidence: uwusweep_core::Confidence::High,
            reason: "test".into(),
            needs_admin: false,
        }
    }

    #[test]
    fn items_match_by_kind_path_and_value() {
        let a = leftover("/opt/example", ItemKind::Dir);
        assert!(same_item(&a, &a.clone()));
        assert!(!same_item(&a, &leftover("/opt/example", ItemKind::File)));
        let mut b = a.clone();
        b.value = Some("x".into());
        assert!(!same_item(&a, &b));
    }

    #[test]
    fn elevated_restores_from_the_own_quarantine_stay_at_home() {
        let item = |kind, original: &str| QuarantineItem {
            kind,
            original: original.into(),
            value: None,
            stored: "files/0".into(),
            size: None,
        };
        let home = Path::new("/home/someone");
        assert!(own_place(
            &item(ItemKind::File, "/home/someone/.app"),
            Some(home)
        ));
        assert!(!own_place(&item(ItemKind::File, "/etc/app"), Some(home)));
        assert!(!own_place(
            &item(ItemKind::File, "/home/someone/.app"),
            None
        ));
        assert!(own_place(
            &item(ItemKind::RegKey, r"HKCU\Software\App"),
            None
        ));
        assert!(!own_place(
            &item(ItemKind::RegKey, r"HKLM\Software\App"),
            None
        ));
        assert!(is_admin_batch("admin-1-x").unwrap() && !is_admin_batch("1-x").unwrap());
        assert!(is_admin_batch("../x").is_err());
    }

    #[test]
    fn only_installers_are_started() {
        let dir = tempfile::tempdir().unwrap();
        let setup = dir.path().join("setup.EXE");
        let script = dir.path().join("run.sh");
        std::fs::write(&setup, b"").unwrap();
        std::fs::write(&script, b"").unwrap();
        assert!(is_installer(&setup));
        assert!(!is_installer(&script));
        assert!(!is_installer(&dir.path().join("missing.msi")));
    }
}
