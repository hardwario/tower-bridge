//! Open at login. The OS registration is the source of truth.

use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_autostart::ManagerExt;
use tracing::{info, warn};

use crate::state::AppState;

pub fn is_enabled(app: &AppHandle) -> bool {
    app.autolaunch().is_enabled().unwrap_or(false)
}

/// Re-register on start so a moved executable or AppImage keeps working.
pub fn refresh(app: &AppHandle) {
    if is_enabled(app)
        && !translocated()
        && let Err(e) = app.autolaunch().enable()
    {
        warn!("cannot refresh login item: {e}");
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
    let r = if enabled {
        app.autolaunch().enable()
    } else {
        app.autolaunch().disable()
    };
    sync(app);
    r.map_err(|e| e.to_string())?;
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
