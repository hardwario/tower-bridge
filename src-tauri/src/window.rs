//! The dashboard window: created on demand, destroyed when closed so the
//! tray-only app keeps a small footprint.

use tauri::window::Color;
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};
use tracing::error;

use crate::settings::ThemePref;
use crate::state::AppState;

pub const DASHBOARD: &str = "dashboard";

const WIDTH: f64 = 1060.0;
/// Tall enough for the sidebar with a dozen modules.
const HEIGHT: f64 = 820.0;
const MIN_WIDTH: f64 = 780.0;
const MIN_HEIGHT: f64 = 560.0;

fn native_theme(pref: ThemePref) -> Option<tauri::Theme> {
    match pref {
        ThemePref::System => None,
        ThemePref::Light => Some(tauri::Theme::Light),
        ThemePref::Dark => Some(tauri::Theme::Dark),
    }
}

/// Window background matching the page, so opening does not flash.
fn background(pref: ThemePref) -> Option<Color> {
    match pref {
        ThemePref::System => None,
        ThemePref::Light => Some(Color(0xf4, 0xf4, 0xf6, 0xff)),
        ThemePref::Dark => Some(Color(0x12, 0x12, 0x16, 0xff)),
    }
}

/// Apply the theme to the native window (title bar, background).
pub fn apply_theme(app: &AppHandle, pref: ThemePref) {
    if let Some(w) = app.get_webview_window(DASHBOARD) {
        let _ = w.set_theme(native_theme(pref));
        let _ = w.set_background_color(background(pref));
    }
}

/// The dashboard's size when it opens: [`WIDTH`] by [`HEIGHT`], but within
/// the screen's work area (without the menu bar, Dock or taskbar), leaving
/// room for the title bar.
fn initial_size(app: &AppHandle) -> (f64, f64) {
    let Some(monitor) = app.primary_monitor().ok().flatten() else {
        return (WIDTH, HEIGHT);
    };
    let area = monitor
        .work_area()
        .size
        .to_logical::<f64>(monitor.scale_factor());
    (
        WIDTH.min(area.width - 40.0).max(MIN_WIDTH),
        HEIGHT.min(area.height - 60.0).max(MIN_HEIGHT),
    )
}

pub fn show_dashboard(app: &AppHandle) {
    if let Some(w) = app.get_webview_window(DASHBOARD) {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
        return;
    }

    #[cfg(windows)]
    if tauri::webview_version().is_err() {
        webview2_missing(app);
        return;
    }

    #[cfg(target_os = "macos")]
    {
        // Show in the Dock and Cmd-Tab while the window is open.
        let _ = app.set_activation_policy(tauri::ActivationPolicy::Regular);
    }

    let (width, height) = initial_size(app);
    let mut builder =
        WebviewWindowBuilder::new(app, DASHBOARD, WebviewUrl::App("index.html".into()))
            .title("TOWER Bridge")
            .inner_size(width, height)
            .min_inner_size(MIN_WIDTH, MIN_HEIGHT)
            .center();
    let theme = app
        .try_state::<AppState>()
        .map(|s| s.settings().theme)
        .unwrap_or_default();
    builder = builder.theme(native_theme(theme));
    if let Some(color) = background(theme) {
        builder = builder.background_color(color);
    }
    if let Some(state) = app.try_state::<AppState>()
        && let Some(dir) = state.paths.webview_dir()
    {
        // WKWebView takes no folder; macOS 14 and newer take a store id.
        #[cfg(target_os = "macos")]
        {
            let mut id = [0u8; 16];
            for (i, b) in dir.to_string_lossy().bytes().enumerate() {
                id[i % 16] = id[i % 16].wrapping_mul(31).wrapping_add(b);
            }
            builder = builder.data_store_identifier(id);
        }
        builder = builder.data_directory(dir);
    }
    match builder.build() {
        Ok(window) => {
            #[cfg(target_os = "macos")]
            {
                let handle = app.clone();
                window.on_window_event(move |event| {
                    if let tauri::WindowEvent::Destroyed = event {
                        let _ = handle.set_activation_policy(tauri::ActivationPolicy::Accessory);
                    }
                });
            }
            let _ = window.set_focus();
        }
        Err(e) => error!("cannot open the dashboard: {e}"),
    }
}

#[cfg(windows)]
fn webview2_missing(app: &AppHandle) {
    use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};
    use tauri_plugin_opener::OpenerExt;

    let handle = app.clone();
    let tr = crate::i18n::current(app).tr();
    app.dialog()
        .message(tr.webview2_missing)
        .title("TOWER Bridge")
        .kind(MessageDialogKind::Warning)
        .buttons(MessageDialogButtons::OkCancelCustom(
            tr.download.into(),
            tr.cancel.into(),
        ))
        .show(move |download| {
            if download {
                let _ = handle.opener().open_url(
                    "https://go.microsoft.com/fwlink/p/?LinkId=2124703",
                    None::<&str>,
                );
            }
        });
}
