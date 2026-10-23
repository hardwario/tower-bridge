//! TOWER Bridge desktop app.

mod autostart;
mod commands;
mod format;
mod i18n;
mod logging;
mod notify;
mod observer;
mod state;
mod tray;
mod update;
mod window;

use std::sync::Arc;
use std::time::Duration;

use tauri::{App, AppHandle, Manager, RunEvent};
use tower_bridge_core::api::ApiInfo;
use tower_bridge_service::{Observer, Paths, Service, ServiceConfig, StartError, settings};
use tracing::error;

use crate::state::AppState;

/// Kept for the life of the app: the log file writer flushes on drop.
struct LogGuard(#[allow(dead_code)] Option<tracing_appender::non_blocking::WorkerGuard>);

pub fn run() {
    let args: Vec<String> = std::env::args().collect();
    let simulated = args.iter().any(|a| a == "--simulate")
        || std::env::var_os("TOWER_BRIDGE_SIMULATE").is_some();
    let autostarted = args.iter().any(|a| a == "--autostart");
    if args.iter().any(|a| a == "--mcp-stdio") {
        // MCP bridge for clients that launch a local process; no GUI.
        std::process::exit(mcp_stdio(simulated));
    }

    // Development instances (simulated bridge or a custom data folder) may run
    // next to an installed copy, so they skip the single-instance check. The
    // service's lock still keeps two instances off one data folder.
    let isolated = simulated || std::env::var_os("TOWER_BRIDGE_DATA_DIR").is_some();
    autostart::set_isolated(isolated);
    let mut builder = tauri::Builder::default();
    if !isolated {
        builder = builder.plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            window::show_dashboard(app);
        }));
    }
    let app = builder
        .plugin(
            tauri_plugin_autostart::Builder::new()
                .args(["--autostart"])
                .build(),
        )
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            commands::get_snapshot,
            commands::get_history,
            commands::get_alert_log,
            commands::save_settings,
            commands::set_open_at_login,
            commands::export_csv,
            commands::export_xlsx,
            commands::send_test_notification,
            commands::identify_device,
            commands::set_relay,
            commands::get_data_range,
            commands::pulse_relay,
            commands::write_nfc_url,
            commands::open_map,
            commands::rescan_device,
            commands::open_data_folder,
            commands::reveal_file,
            commands::open_link,
            commands::check_for_updates,
            commands::open_release,
        ])
        .setup(move |app| {
            if let Err(e) = setup(app, simulated, autostarted) {
                error!("startup failed: {e:#}");
                fatal(app.handle(), &format!("{e:#}"));
            }
            Ok(())
        })
        .build(tauri::generate_context!());

    let app = match app {
        Ok(app) => app,
        Err(e) => {
            eprintln!("TOWER Bridge failed to start: {e}");
            std::process::exit(1);
        }
    };

    app.run(|app, event| match event {
        // Closing the last window keeps the tray app running.
        RunEvent::ExitRequested {
            code: None, api, ..
        } => api.prevent_exit(),
        // Long enough for a measurement cycle to end, so the sensors are
        // stopped and the relay is switched back from a pulse.
        RunEvent::Exit => shutdown(app, Duration::from_secs(4)),
        _ => {}
    });
}

/// A start that cannot go on: say why in a message box (the Windows build has
/// no console, and the log may not exist yet), then quit. Returning the
/// error to Tauri instead would end in a panic without a word.
fn fatal(app: &tauri::AppHandle, error: &str) {
    use tauri_plugin_dialog::{DialogExt, MessageDialogKind};
    let tr = i18n::Lang::effective(settings::LanguagePref::System).tr();
    let dialog = app
        .dialog()
        .message(i18n::fill(tr.cannot_start, &[("error", error)]))
        .kind(MessageDialogKind::Error)
        .title("TOWER Bridge");
    // On Linux a blocking dialog on the main thread, which owns GTK, could
    // hang: there it shows once the event loop runs and quits when closed.
    if cfg!(target_os = "linux") {
        dialog.show(|_| std::process::exit(1));
    } else {
        dialog.blocking_show();
        std::process::exit(1);
    }
}

fn setup(app: &mut App, simulated: bool, autostarted: bool) -> anyhow::Result<()> {
    #[cfg(target_os = "macos")]
    app.set_activation_policy(tauri::ActivationPolicy::Accessory);

    let handle = app.handle().clone();
    let paths =
        Paths::resolve(None, simulated).ok_or_else(|| anyhow::anyhow!("no local data folder"))?;
    paths.create()?;
    app.manage(LogGuard(logging::init(&paths.logs_dir())));

    // The service first: an instance refused for the lock shows no tray.
    let observer = Arc::new(observer::AppObserver::new(handle.clone()));
    let service = Service::start(ServiceConfig {
        paths: paths.clone(),
        mode: tower_bridge_service::Mode::Full,
        simulated,
        program: "tower-bridge".into(),
        version: handle.package_info().version.to_string(),
        observer: observer.clone(),
    })
    .map_err(|e| match e {
        // The CLI's service, or another development instance, runs with
        // the same data folder.
        StartError::AlreadyRunning(holder, _) => {
            let tr = i18n::Lang::effective(settings::LanguagePref::System).tr();
            anyhow::anyhow!(match holder.program {
                Some(program) => i18n::fill(tr.already_running, &[("holder", &program)]),
                None => tr.already_running_other.to_string(),
            })
        }
        e => e.into(),
    })?;
    let tray = tray::TrayController::build(&handle)?;
    observer.set_tray(tray.clone());
    observer.refresh(service.state());
    let first_run = service.first_run;
    app.manage(AppState {
        service,
        tray: tray.clone(),
    });

    app.manage(update::Updates::start(&handle, paths.data_dir.clone()));

    autostart::refresh(&handle);
    tray.set_open_at_login(autostart::is_enabled(&handle));
    #[cfg(windows)]
    notify::register_aumid(&handle, &paths.data_dir);
    // On macOS asks for permission now, at the start (the user's choice):
    // the prompt appears the first time only.
    notify::init(&handle);

    if first_run && !autostarted {
        window::show_dashboard(&handle);
    }
    Ok(())
}

/// `tower-bridge --mcp-stdio`: MCP over stdin/stdout, forwarding to the
/// running app. Runs without starting the GUI.
fn mcp_stdio(simulated: bool) -> i32 {
    let Some(paths) = Paths::resolve(None, simulated) else {
        eprintln!("tower-bridge: cannot determine the data folder");
        return 1;
    };
    let info = ApiInfo {
        name: "tower-bridge".into(),
        version: env!("CARGO_PKG_VERSION").into(),
    };
    match tower_bridge_service::api::mcp_stdio(&paths, info) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("tower-bridge: {e}");
            1
        }
    }
}

/// Stop background threads (LED off, device closed).
pub(crate) fn shutdown(app: &AppHandle, timeout: Duration) {
    if let Some(state) = app.try_state::<AppState>() {
        state.service.shutdown(timeout);
    }
}
