//! Where settings, the database and logs live.
//!
//! Portable mode (Windows): when a `data` folder exists next to the
//! executable, everything (including the WebView2 profile) is stored there.
//! `TOWER_BRIDGE_DATA_DIR` overrides the location on every platform.

use std::path::PathBuf;

use serde::Serialize;
use tauri::{AppHandle, Manager};

/// Bundle identifier (must match `identifier` in tauri.conf.json).
pub const IDENTIFIER: &str = "com.hardwario.tower-bridge";

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct Paths {
    pub data_dir: PathBuf,
    pub portable: bool,
}

impl Paths {
    pub fn resolve(app: &AppHandle, simulated: bool) -> anyhow::Result<Self> {
        debug_assert_eq!(app.config().identifier, IDENTIFIER);
        let default = app.path().app_local_data_dir()?;
        let paths = Self::locate(default, simulated);
        std::fs::create_dir_all(&paths.data_dir)?;
        Ok(paths)
    }

    /// Resolve without a running Tauri app (the `--mcp-stdio` bridge), using
    /// the same rule as Tauri's `app_local_data_dir`.
    pub fn resolve_standalone(simulated: bool) -> Option<Self> {
        let default = dirs::data_local_dir()?.join(IDENTIFIER);
        Some(Self::locate(default, simulated))
    }

    fn locate(default: PathBuf, simulated: bool) -> Self {
        let (mut data_dir, portable) = if let Some(dir) = std::env::var_os("TOWER_BRIDGE_DATA_DIR")
        {
            (PathBuf::from(dir), false)
        } else if let Some(dir) = portable_dir() {
            (dir, true)
        } else {
            (default, false)
        };
        if simulated {
            // Keep simulated readings out of the real history.
            data_dir = data_dir.join("simulated");
        }
        Self { data_dir, portable }
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

    /// WebView profile directory in portable mode.
    pub fn webview_dir(&self) -> Option<PathBuf> {
        self.portable.then(|| self.data_dir.join("webview"))
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
