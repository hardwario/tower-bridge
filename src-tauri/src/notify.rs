//! Native notifications: the notification plugin on Windows and Linux, the
//! UserNotifications framework on macOS (in an app bundle; development
//! builds keep the plugin).

#[cfg(target_os = "macos")]
mod macos;

use tauri::AppHandle;
use tauri_plugin_notification::NotificationExt;
use tracing::warn;

/// At start: on macOS ask for permission (macOS prompts only the first time)
/// and show notifications also while the dashboard is in front.
pub fn init(app: &AppHandle) {
    #[cfg(target_os = "macos")]
    if macos::available() {
        macos::init(app);
    }
    #[cfg(not(target_os = "macos"))]
    let _ = app;
}

pub fn send(app: &AppHandle, title: &str, body: &str) {
    #[cfg(target_os = "macos")]
    if macos::available() {
        macos::send(title, body);
        return;
    }
    if let Err(e) = app.notification().builder().title(title).body(body).show() {
        warn!("notification failed: {e}");
    }
}

/// Whether the system has notifications turned off for TOWER Bridge (macOS:
/// in System Settings, Notifications).
pub fn blocked() -> bool {
    #[cfg(target_os = "macos")]
    if macos::available() {
        return macos::blocked();
    }
    false
}

/// Windows shows toasts from an unpackaged exe only when its AppUserModelID
/// is registered. The notification plugin uses the bundle identifier, so
/// register it under HKCU (no admin rights needed).
#[cfg(windows)]
pub fn register_aumid(app: &AppHandle, data_dir: &std::path::Path) {
    use winreg::RegKey;
    use winreg::enums::HKEY_CURRENT_USER;

    static ICON: &[u8] = include_bytes!("../icons/128x128.png");

    let id = app.config().identifier.clone();
    let icon_path = data_dir.join("icon.png");
    if !icon_path.exists() {
        let _ = std::fs::write(&icon_path, ICON);
    }
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    match hkcu.create_subkey(format!(r"Software\Classes\AppUserModelId\{id}")) {
        Ok((key, _)) => {
            let _ = key.set_value("DisplayName", &"TOWER Bridge");
            let _ = key.set_value("IconUri", &icon_path.display().to_string());
        }
        Err(e) => warn!("cannot register notification identity: {e}"),
    }
}
