//! The log: to the console, and to a file that can go with an issue.
//!
//! Release builds on Windows have no console, and an app started from the
//! Dock or the Start menu has nobody watching its output; so everything goes
//! into `uwusweep.log` in the app's log folder too, the previous start's in
//! `uwusweep.old.log`. The page can turn on the detailed log (every step
//! of a scan) without a restart. `RUST_LOG`, when set, wins.

use std::fs::File;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};

use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{fmt, reload, EnvFilter, Registry};

const NORMAL: &str = "info";
/// Ours in detail; the libraries underneath would drown it.
const DETAILED: &str = "debug,tao=info,wry=info,tauri=info";

pub const FILE: &str = "uwusweep.log";
const OLD_FILE: &str = "uwusweep.old.log";

/// Swaps the filter; unset when `RUST_LOG` decides.
static FILTER: OnceLock<reload::Handle<EnvFilter, Registry>> = OnceLock::new();
static DETAILED_ON: AtomicBool = AtomicBool::new(false);

fn open(dir: &Path) -> std::io::Result<File> {
    std::fs::create_dir_all(dir)?;
    let _ = std::fs::rename(dir.join(FILE), dir.join(OLD_FILE));
    File::create(dir.join(FILE))
}

pub fn init(dir: &Path) {
    let from_env = EnvFilter::try_from_default_env().ok();
    let fixed = from_env.is_some();
    let (filter, handle) = reload::Layer::new(from_env.unwrap_or_else(|| EnvFilter::new(NORMAL)));
    let (file, file_error) = match open(dir) {
        Ok(file) => (Some(file), None),
        Err(error) => (None, Some(error)),
    };
    let file_layer = file.map(|file| fmt::layer().with_ansi(false).with_writer(Mutex::new(file)));
    let result = tracing_subscriber::registry()
        .with(filter)
        .with(fmt::layer())
        .with(file_layer)
        .try_init();
    if result.is_err() {
        return;
    }
    if !fixed {
        let _ = FILTER.set(handle);
    }
    if let Some(error) = file_error {
        tracing::warn!(dir = %dir.display(), %error, "no log file");
    }
    tracing::info!(
        version = env!("CARGO_PKG_VERSION"),
        os = std::env::consts::OS,
        arch = std::env::consts::ARCH,
        "UwUSweep starting"
    );
}

/// The detailed log on or off, from now on.
pub fn set_detailed(on: bool) {
    let Some(handle) = FILTER.get() else { return };
    if DETAILED_ON.swap(on, Ordering::Relaxed) == on {
        return;
    }
    if handle
        .reload(EnvFilter::new(if on { DETAILED } else { NORMAL }))
        .is_ok()
    {
        tracing::info!(detailed = on, "log");
    }
}
