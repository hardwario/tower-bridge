//! macOS 13 and newer: the app itself as login item (SMAppService), listed
//! by name and icon under Open at Login. The launch agent the plugin writes
//! shows as "HARDWARIO a.s., 1 item" under Allow in the Background instead.
#![allow(unsafe_code)]

use objc2_foundation::NSProcessInfo;
use objc2_service_management::{SMAppService, SMAppServiceStatus};

/// An app bundle on macOS 13 or newer. An unbundled build (`cargo run`) has
/// no app to register and keeps the launch agent, as do macOS 11 and 12.
pub fn available() -> bool {
    std::env::current_exe().is_ok_and(|p| p.to_string_lossy().contains(".app/Contents/MacOS/"))
        && NSProcessInfo::processInfo()
            .operatingSystemVersion()
            .majorVersion
            >= 13
}

fn service() -> objc2::rc::Retained<SMAppService> {
    // SAFETY: SMAppService exists from macOS 13, which `available` checks.
    unsafe { SMAppService::mainAppService() }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Status {
    Enabled,
    /// Registered, but switched off in System Settings.
    RequiresApproval,
    Off,
}

pub fn status() -> Status {
    // SAFETY: a plain property read.
    match unsafe { service().status() } {
        SMAppServiceStatus::Enabled => Status::Enabled,
        SMAppServiceStatus::RequiresApproval => Status::RequiresApproval,
        _ => Status::Off,
    }
}

pub fn register() -> Result<(), String> {
    // SAFETY: registers the running app, signed as it must be.
    unsafe { service().registerAndReturnError() }.map_err(|e| e.localizedDescription().to_string())
}

pub fn unregister() -> Result<(), String> {
    if status() == Status::Off {
        return Ok(());
    }
    // SAFETY: unregisters the running app.
    unsafe { service().unregisterAndReturnError() }
        .map_err(|e| e.localizedDescription().to_string())
}

/// System Settings, General, Login Items: where a switched-off item is
/// allowed again.
pub fn open_settings() {
    // SAFETY: opens a System Settings pane.
    unsafe { SMAppService::openSystemSettingsLoginItems() }
}
