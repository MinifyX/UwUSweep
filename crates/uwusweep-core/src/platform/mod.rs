//! The operating system side. Every module exports the same functions; the
//! rest of the crate and the app only call them through this module:
//!
//! - `list_apps() -> Result<Vec<App>>` — installed programs, sorted by name.
//! - `leftover_roots(level) -> Vec<scan::Root>` — folders the file leftover
//!   search walks, with depth and whether touching them needs admin.
//! - `registry_leftovers(&Terms, level, &App) -> Vec<Leftover>` — Windows
//!   registry keys and values; empty elsewhere.
//! - `uninstall(&App, quiet) -> Result<()>` — runs the app's own uninstaller
//!   (or package manager) and returns when it has finished.
//! - `autostart_list() -> Result<Vec<AutostartEntry>>` and
//!   `autostart_set(&AutostartEntry, enabled) -> Result<()>`.
//! - `create_restore_point(description) -> Result<()>` — Windows only,
//!   `Error::Unsupported` elsewhere. Needs admin.
//! - `quarantine_registry(&Leftover, dest: &Path) -> Result<()>` and
//!   `restore_registry(&QuarantineItem, src: &Path) -> Result<()>` — export a
//!   key or value to JSON and delete it, and back. `Unsupported` off Windows.
//! - `registry_snapshot() -> BTreeSet<String>` — key paths for install traces;
//!   empty off Windows.
//! - `is_elevated() -> bool`.
//! - `run_elevated(exe: &Path, args: &[String]) -> Result<i32>` — starts a
//!   program with admin rights (UAC, the macOS admin prompt, pkexec), waits,
//!   returns its exit code; `Error::Cancelled` when the user declines.
//! - `admin_quarantine_root() -> PathBuf` — the elevated helper's own
//!   quarantine, in a system folder only admins can change.
//! - `secure_dir(path, private) -> Result<()>` — creates a folder there, or
//!   checks an existing one, and locks it to admins (`private`: admins only,
//!   else readable by users too).

#[cfg(unix)]
mod unix;
#[cfg(unix)]
pub use unix::secure_dir;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub use linux::*;

#[cfg(target_os = "macos")]
mod macos;
// The pure parsers, tested on every system.
#[cfg(any(target_os = "macos", test))]
mod macos_parse;
#[cfg(target_os = "macos")]
pub use macos::*;

#[cfg(windows)]
mod windows;
#[cfg(any(windows, test))]
mod windows_parse;
#[cfg(windows)]
pub use windows::*;
