//! Where settings, the database and logs live: one data folder, shared by the
//! desktop app and the CLI.
//!
//! In order:
//! - a folder given on the command line (the CLI's `--data-dir`);
//! - `TOWER_BRIDGE_DATA_DIR`;
//! - portable mode (Windows): a `data` folder next to the executable;
//! - the platform's local data folder (the same as Tauri's
//!   `app_local_data_dir`): `~/Library/Application Support/<id>` on macOS,
//!   `%LOCALAPPDATA%\<id>` on Windows, `~/.local/share/<id>` on Linux.
//!
//! A simulated bridge keeps its data in `simulated/` below it, apart from
//! real readings.

use std::path::PathBuf;

use serde::Serialize;

/// Bundle identifier (must match `identifier` in tauri.conf.json).
pub const IDENTIFIER: &str = "com.hardwario.tower-bridge";

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct Paths {
    pub data_dir: PathBuf,
    pub portable: bool,
    /// Not the platform's folder (`--data-dir`, `TOWER_BRIDGE_DATA_DIR`, a
    /// simulated bridge): a development instance next to the real one.
    #[serde(skip)]
    pub custom: bool,
}

impl Paths {
    /// The data folder; `None` only when the platform has no local data folder.
    pub fn resolve(dir: Option<PathBuf>, simulated: bool) -> Option<Self> {
        let given = dir.or_else(|| std::env::var_os("TOWER_BRIDGE_DATA_DIR").map(PathBuf::from));
        let custom = given.is_some() || simulated;
        let (mut data_dir, portable) = if let Some(dir) = given {
            (dir, false)
        } else if let Some(dir) = portable_dir() {
            (dir, true)
        } else {
            (dirs::data_local_dir()?.join(IDENTIFIER), false)
        };
        if simulated {
            // Keep simulated readings out of the real history.
            data_dir = data_dir.join("simulated");
        }
        Some(Self {
            data_dir,
            portable,
            custom,
        })
    }

    pub fn create(&self) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.data_dir)
    }

    pub fn settings_file(&self) -> PathBuf {
        self.data_dir.join("settings.json")
    }

    pub fn db_file(&self) -> PathBuf {
        self.data_dir.join("tower-bridge.db")
    }

    pub fn logs_dir(&self) -> PathBuf {
        self.data_dir.join("logs")
    }

    /// Held by the running instance (see `lock`).
    pub fn lock_file(&self) -> PathBuf {
        self.data_dir.join("tower-bridge.lock")
    }

    /// A WebView profile of its own, in portable mode and for development
    /// instances (which would otherwise share the real one's local storage).
    pub fn webview_dir(&self) -> Option<PathBuf> {
        (self.portable || self.custom).then(|| self.data_dir.join("webview"))
    }
}

#[cfg(windows)]
fn portable_dir() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?.join("data");
    dir.is_dir().then_some(dir)
}

#[cfg(not(windows))]
fn portable_dir() -> Option<PathBuf> {
    None
}
