//! UwUSweep's core: which programs are installed, removing them with their
//! own uninstaller, then finding what they left behind and moving that into a
//! quarantine that can be undone. Also install traces (a snapshot before and
//! after an install) and the autostart list.
//!
//! The shared logic is plain Rust and tested on every system. Everything that
//! asks the operating system lives in [`platform`], one module per system,
//! each with the same functions.

pub mod autostart;
pub mod helper;
pub mod model;
pub mod platform;
pub mod quarantine;
pub mod scan;
pub mod snapshot;
pub mod terms;

pub use model::*;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    /// Not on this system (a restore point on Linux…).
    #[error("not supported on this system")]
    Unsupported,
    /// The user said no to the admin prompt, or closed the uninstaller.
    #[error("cancelled")]
    Cancelled,
    #[error("{0}")]
    Failed(String),
}

pub type Result<T> = std::result::Result<T, Error>;

/// Seconds since the Unix epoch.
pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
