//! TOWER Bridge desktop app.

mod actions;
mod api_ctl;
mod autostart;
mod commands;
mod format;
mod hub;
mod i18n;
mod logging;
mod notify;
mod paths;
mod settings;
mod state;
mod tray;
mod window;

use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use parking_lot::{Mutex, RwLock};
use tauri::{App, AppHandle, Manager, RunEvent};
use tower_bridge_core::acquisition::{self, AcqConfig, Acquisition};
use tower_bridge_core::bus::SystemClock;
use tower_bridge_core::model::DeviceState;
use tower_bridge_core::store::Store;
use tracing::{error, info};

use crate::hub::HubMsg;
use crate::paths::Paths;
use crate::settings::Settings;
use crate::state::{ApiState, AppState, MqttState};

/// Background threads, stopped on exit.
struct Workers {
    acquisition: Mutex<Option<Acquisition>>,
    hub: Mutex<Option<JoinHandle<()>>>,
    _log_guard: Option<tracing_appender::non_blocking::WorkerGuard>,
}

pub fn run() {
    let args: Vec<String> = std::env::args().collect();
    let simulated = args.iter().any(|a| a == "--simulate")
        || std::env::var_os("TOWER_BRIDGE_SIMULATE").is_some();
    let autostarted = args.iter().any(|a| a == "--autostart");
    if args.iter().any(|a| a == "--mcp-stdio") {
        // MCP bridge for clients that launch a local process; no GUI.
        std::process::exit(api_ctl::run_mcp_stdio(simulated));
    }

    // Development instances (simulated bridge or a custom data folder) may run
    // next to an installed copy, so they skip the single-instance check.
    let isolated = simulated || std::env::var_os("TOWER_BRIDGE_DATA_DIR").is_some();
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
        ])
        .setup(move |app| {
            setup(app, simulated, autostarted).map_err(|e| {
                error!("startup failed: {e:#}");
                e.into()
            })
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
        RunEvent::Exit => shutdown(app, Duration::from_secs(1)),
        _ => {}
    });
}

fn setup(app: &mut App, simulated: bool, autostarted: bool) -> anyhow::Result<()> {
    #[cfg(target_os = "macos")]
    app.set_activation_policy(tauri::ActivationPolicy::Accessory);

    let handle = app.handle().clone();
    let paths = Paths::resolve(&handle, simulated)?;
    let log_guard = logging::init(&paths.logs_dir());
    info!(
        "TOWER Bridge {} starting (data: {}{}{})",
        handle.package_info().version,
        paths.data_dir.display(),
        if paths.portable { ", portable" } else { "" },
        if simulated { ", simulated bridge" } else { "" },
    );

    let (settings, first_run) = Settings::load(&paths.settings_file());
    // Persist defaults added by newer versions (and the generated API token).
    if let Err(e) = settings.save(&paths.settings_file()) {
        error!("cannot write settings: {e}");
    }
    let settings = Arc::new(settings);

    let store = match Store::open(&paths.db_file()) {
        Ok(mut s) => {
            if simulated {
                seed_simulated_history(&mut s, settings.sample_interval_s);
            }
            Some(s)
        }
        Err(e) => {
            error!("cannot open the database, history is disabled: {e}");
            None
        }
    };

    let tray = tray::TrayController::build(&handle)?;

    let (acq_tx, acq_rx) = crossbeam_channel::unbounded();
    let (hub_tx, hub_rx) = crossbeam_channel::unbounded();
    let (mqtt_cmd_tx, mqtt_cmd_rx) = crossbeam_channel::unbounded();
    let opener = if simulated {
        acquisition::simulated_opener()
    } else {
        acquisition::hardware_opener(settings.backend)
    };
    let acq = Acquisition::spawn(
        AcqConfig {
            interval: Duration::from_secs(settings.sample_interval_s),
            channel: settings.i2c_channel,
            state_dir: Some(paths.data_dir.clone()),
        },
        opener,
        Box::new(SystemClock::new()),
        acq_tx,
    )?;

    app.manage(AppState {
        settings: RwLock::new(settings.clone()),
        settings_lock: parking_lot::Mutex::new(()),
        latest: RwLock::new(Vec::new()),
        device: RwLock::new(DeviceState::Searching),
        relay: RwLock::new(None),
        nfc: RwLock::new(None),
        gps: RwLock::new(None),
        alerts: RwLock::new(Vec::new()),
        mqtt: RwLock::new(MqttState::Disabled),
        api: RwLock::new(ApiState::Disabled),
        paths: paths.clone(),
        simulated,
        acq_tx: acq.sender(),
        hub_tx: hub_tx.clone(),
        tray: tray.clone(),
    });
    app.manage(api_ctl::ApiController::default());
    api_ctl::apply(&handle, &settings);
    actions::spawn_mqtt_commands(handle.clone(), mqtt_cmd_rx)?;
    let hub = hub::spawn(handle.clone(), acq_rx, hub_rx, store, settings, mqtt_cmd_tx)?;
    app.manage(Workers {
        acquisition: Mutex::new(Some(acq)),
        hub: Mutex::new(Some(hub)),
        _log_guard: log_guard,
    });

    autostart::refresh(&handle);
    tray.set_open_at_login(autostart::is_enabled(&handle));
    #[cfg(windows)]
    notify::register_aumid(&handle, &paths.data_dir);
    let _ = hub_tx.send(HubMsg::Init);

    if first_run && !autostarted {
        window::show_dashboard(&handle);
    }
    Ok(())
}

/// Development aid: with `TOWER_BRIDGE_SEED_DAYS=<n>` in simulation mode,
/// fill an empty database with `n` days of generated readings.
fn seed_simulated_history(store: &mut Store, interval_s: u64) {
    let Some(days) = std::env::var("TOWER_BRIDGE_SEED_DAYS")
        .ok()
        .and_then(|v| v.parse::<i64>().ok())
        .filter(|d| (1..=400).contains(d))
    else {
        return;
    };
    if !matches!(tower_bridge_core::store::first_ts(store.conn()), Ok(None)) {
        return;
    }
    let step = interval_s.max(2) as i64 * 1000;
    let now = tower_bridge_core::acquisition::unix_ms();
    let mut seed = 0x9E37_79B9_7F4A_7C15u64;
    let mut noise = || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed >> 11) as f64 / (1u64 << 53) as f64 - 0.5
    };
    let mut batch = Vec::with_capacity(10_000);
    let mut ts = now - days * 86_400_000;
    while ts < now {
        // The simulated Climate Module on channel 0 and tags on channel 1.
        for channel in [0u8, 1] {
            let n = [noise(), noise(), noise(), noise()];
            let wall = ts as f64 / 1000.0;
            let mut values = tower_bridge_core::bus::mock::simulated_values(
                wall - 1800.0 * f64::from(channel),
                n,
            );
            if channel == 1 {
                values.temperature = values.temperature.map(|t| t - 3.5);
                values.humidity = values.humidity.map(|h| (h + 9.0).min(100.0));
                values.illuminance = values.illuminance.map(|l| l * 1.8);
                values.pressure = values.pressure.map(|p| p + 12.0);
            }
            batch.push(tower_bridge_core::model::Reading {
                ts,
                device: "SIMULATED".into(),
                channel,
                values,
                altitude: None,
            });
        }
        if batch.len() == 10_000 {
            let _ = store.insert_many(&batch);
            batch.clear();
        }
        ts += step;
    }
    let _ = store.insert_many(&batch);
    info!("seeded {days} days of simulated history");
}

/// Stop background threads (LED off, device closed).
pub(crate) fn shutdown(app: &AppHandle, timeout: Duration) {
    let Some(workers) = app.try_state::<Workers>() else {
        return;
    };
    if let Some(state) = app.try_state::<AppState>() {
        let _ = state.hub_tx.send(HubMsg::Shutdown);
    }
    api_ctl::stop(app);
    if let Some(acq) = workers.acquisition.lock().take() {
        acq.shutdown(timeout);
    }
    if let Some(hub) = workers.hub.lock().take() {
        let deadline = std::time::Instant::now() + timeout;
        while !hub.is_finished() && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    info!("stopped");
}
