//! Open at login. The OS registration is the source of truth. On macOS 13
//! and newer an app bundle registers itself as login item (SMAppService);
//! elsewhere, and for development builds, the plugin's launch agent, Run
//! key or desktop entry.

#[cfg(target_os = "macos")]
mod macos;

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

/// The plugin's registration (on macOS the launch agent).
fn agent_enabled(app: &AppHandle) -> bool {
    app.autolaunch().is_enabled().unwrap_or(false)
}

pub fn is_enabled(app: &AppHandle) -> bool {
    #[cfg(target_os = "macos")]
    if macos::available() && macos::status() == macos::Status::Enabled {
        return true;
    }
    agent_enabled(app)
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

/// Whether the registration belongs to this program: the one recorded, or a
/// moved one (the recorded program is gone), or for a registration by a
/// version that kept no record, anything but a development build. Any other
/// copy (a newer download, a portable exe elsewhere, a development build)
/// would take the login item over.
fn ours(app: &AppHandle, program: &Path) -> bool {
    let recorded = record(app)
        .and_then(|f| std::fs::read_to_string(f).ok())
        .map(|s| PathBuf::from(s.trim()));
    match &recorded {
        Some(path) => path == program || !path.exists(),
        None => !development_build(program),
    }
}

/// Re-register on start so a moved executable or AppImage keeps working, but
/// only for the program that turned it on. On macOS 13+ the launch agent of
/// 1.0.0 is replaced by the login item instead (which follows the app by
/// itself).
pub fn refresh(app: &AppHandle) {
    if !agent_enabled(app) || isolated() || translocated() {
        return;
    }
    let Some(program) = program() else {
        return;
    };
    if !ours(app, &program) {
        return;
    }
    #[cfg(target_os = "macos")]
    if macos::available() {
        match macos::register() {
            Ok(()) => {
                if let Err(e) = app.autolaunch().disable() {
                    warn!("cannot remove the launch agent: {e}");
                }
                remember(app, Some(&program));
                info!("open at login: the launch agent is now a login item");
            }
            // Kept: it still opens the app at login.
            Err(e) => warn!("cannot register the login item, keeping the launch agent: {e}"),
        }
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

/// Turn Open at login on or off with the plugin.
fn set_agent(app: &AppHandle, enabled: bool) -> Result<(), String> {
    let r = if enabled {
        app.autolaunch().enable()
    } else {
        app.autolaunch().disable()
    };
    r.map_err(|e| e.to_string())
}

/// Turn Open at login on or off with the login item, falling back to the
/// launch agent when macOS refuses to register it (an unsigned build).
#[cfg(target_os = "macos")]
fn set_login_item(app: &AppHandle, enabled: bool) -> Result<(), String> {
    if !enabled {
        let r = macos::unregister();
        if agent_enabled(app) {
            set_agent(app, false)?;
        }
        return r;
    }
    match macos::register() {
        Ok(()) => {
            // The launch agent of 1.0.0 would open the app a second time.
            if agent_enabled(app) {
                set_agent(app, false)?;
            }
            // Switched off in System Settings: register succeeds, but only
            // the user can allow it again there.
            if macos::status() == macos::Status::RequiresApproval {
                macos::open_settings();
            }
            Ok(())
        }
        Err(e) => {
            warn!("cannot register the login item, using a launch agent: {e}");
            set_agent(app, true)
        }
    }
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
    #[cfg(target_os = "macos")]
    let r = if macos::available() {
        set_login_item(app, enabled)
    } else {
        set_agent(app, enabled)
    };
    #[cfg(not(target_os = "macos"))]
    let r = set_agent(app, enabled);
    sync(app);
    r?;
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
