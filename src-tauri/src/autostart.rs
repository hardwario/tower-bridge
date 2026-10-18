//! Open at login. The OS registration is the source of truth.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_autostart::ManagerExt;
use tracing::{info, warn};

use crate::state::AppState;

/// A development instance (simulated bridge or another data folder) must
/// never become the login item: at the next login it would start in place of
/// the installed copy, with the real data and the bridge.
static ISOLATED: AtomicBool = AtomicBool::new(false);

pub fn set_isolated(isolated: bool) {
    ISOLATED.store(isolated, Ordering::Relaxed);
}

fn isolated() -> bool {
    ISOLATED.load(Ordering::Relaxed)
}

pub fn is_enabled(app: &AppHandle) -> bool {
    app.autolaunch().is_enabled().unwrap_or(false)
}

/// This program as the login item names it (an AppImage by its own path).
fn program() -> Option<PathBuf> {
    std::env::var_os("APPIMAGE")
        .map(PathBuf::from)
        .or_else(|| std::env::current_exe().ok())
}

/// Where the program that turned Open at login on is recorded.
fn record(app: &AppHandle) -> Option<PathBuf> {
    app.try_state::<AppState>()
        .map(|s| s.paths.data_dir.join("open-at-login.txt"))
}

/// A build of the source tree (`cargo build`, `npm run tauri dev`).
fn development_build(program: &Path) -> bool {
    let p = program.to_string_lossy().replace('\\', "/");
    p.contains("/target/debug/") || p.contains("/target/release/")
}

/// Re-register on start so a moved executable or AppImage keeps working,
/// but only for the program that turned it on: any other copy (a newer
/// download, a portable exe elsewhere, a development build) would take the
/// login item over.
pub fn refresh(app: &AppHandle) {
    if !is_enabled(app) || isolated() || translocated() {
        return;
    }
    let Some(program) = program() else {
        return;
    };
    let recorded = record(app)
        .and_then(|f| std::fs::read_to_string(f).ok())
        .map(|s| PathBuf::from(s.trim()));
    let ours = match &recorded {
        // Moved: the program recorded is gone.
        Some(path) => path == &program || !path.exists(),
        // Turned on by an older version, which kept no record.
        None => !development_build(&program),
    };
    if !ours {
        return;
    }
    match app.autolaunch().enable() {
        Ok(()) => remember(app, Some(&program)),
        Err(e) => warn!("cannot refresh login item: {e}"),
    }
}

fn remember(app: &AppHandle, program: Option<&Path>) {
    let Some(file) = record(app) else {
        return;
    };
    let result = match program {
        Some(p) => std::fs::write(&file, p.to_string_lossy().as_bytes()),
        None => std::fs::remove_file(&file).or_else(|e| match e.kind() {
            std::io::ErrorKind::NotFound => Ok(()),
            _ => Err(e),
        }),
    };
    if let Err(e) = result {
        warn!("cannot record the login item in {}: {e}", file.display());
    }
}

/// macOS runs quarantined apps from a randomized read-only path; a login
/// item pointing there would break.
fn translocated() -> bool {
    cfg!(target_os = "macos")
        && std::env::current_exe()
            .map(|p| p.to_string_lossy().contains("/AppTranslocation/"))
            .unwrap_or(false)
}

pub fn set(app: &AppHandle, enabled: bool) -> Result<bool, String> {
    if enabled && translocated() {
        sync(app);
        return Err(crate::i18n::current(app).tr().move_to_applications.into());
    }
    if isolated() {
        sync(app);
        return Err(crate::i18n::current(app).tr().login_in_dev_instance.into());
    }
    let r = if enabled {
        app.autolaunch().enable()
    } else {
        app.autolaunch().disable()
    };
    sync(app);
    r.map_err(|e| e.to_string())?;
    remember(app, if enabled { program() } else { None }.as_deref());
    let now = is_enabled(app);
    info!("open at login: {now}");
    Ok(now)
}

fn sync(app: &AppHandle) {
    let now = is_enabled(app);
    if let Some(state) = app.try_state::<AppState>() {
        state.tray.set_open_at_login(now);
    }
    let _ = app.emit("open-at-login-changed", now);
}

pub fn toggle(app: &AppHandle) {
    if let Err(e) = set(app, !is_enabled(app)) {
        crate::notify::send(app, crate::i18n::current(app).tr().open_at_login, &e);
    }
}
