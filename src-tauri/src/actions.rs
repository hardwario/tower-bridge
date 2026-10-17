//! Operations shared by commands and the tray menu.

use std::sync::Arc;
use std::time::Duration;

use crossbeam_channel::Receiver;
use tauri::{AppHandle, Emitter, Manager};
use tower_bridge_core::acquisition::{AcqCommand, AcqError, Reply, RescanOutcome};
use tower_bridge_core::mqtt::MqttCommand;
use tower_bridge_core::relay::RelayInfo;
use tracing::warn;

use crate::hub::HubMsg;
use crate::settings::Settings;
use crate::state::{AppState, MqttState};

/// Validate, persist and apply new settings.
pub fn apply_settings(app: &AppHandle, new: Settings) -> std::io::Result<Arc<Settings>> {
    let state = app.state::<AppState>();
    let new = Arc::new(new.validated());
    new.save(&state.paths.settings_file())?;
    let old = std::mem::replace(&mut *state.settings.write(), new.clone());
    if old.sample_interval_s != new.sample_interval_s {
        let _ = state
            .acq_tx
            .send(AcqCommand::SetInterval(Duration::from_secs(
                new.sample_interval_s,
            )));
    }
    if old.theme != new.theme {
        crate::window::apply_theme(app, new.theme);
    }
    if old.i2c_channel != new.i2c_channel {
        let _ = state.acq_tx.send(AcqCommand::SetChannel(new.i2c_channel));
    }
    let api_changed = old.api != new.api;
    let _ = state.hub_tx.send(HubMsg::SettingsChanged {
        old,
        new: new.clone(),
    });
    if api_changed {
        crate::api_ctl::apply(app, &new);
    }
    let _ = app.emit("settings-changed", &*new);
    Ok(new)
}

pub fn toggle_mqtt(app: &AppHandle) {
    let state = app.state::<AppState>();
    let mut s = (*state.settings()).clone();
    s.mqtt.enabled = !s.mqtt.enabled;
    let was_running = matches!(*state.mqtt.read(), MqttState::Running { .. });
    let tr = crate::i18n::current(app).tr();
    if let Err(e) = apply_settings(app, s.clone()) {
        let msg = crate::i18n::fill(tr.cannot_save_settings, &[("error", &e.to_string())]);
        crate::notify::send(app, tr.mqtt_broker, &msg);
        return;
    }
    if !s.mqtt.enabled && was_running {
        ask_restart(app, tr.restart_to_stop_broker);
    }
}

/// Offer to restart the app (needed to stop or re-bind the MQTT broker).
pub fn ask_restart(app: &AppHandle, message: &str) {
    use tauri_plugin_dialog::{DialogExt, MessageDialogButtons};
    let handle = app.clone();
    let tr = crate::i18n::current(app).tr();
    app.dialog()
        .message(message)
        .title("TOWER Bridge")
        .buttons(MessageDialogButtons::OkCancelCustom(
            tr.restart.into(),
            tr.later.into(),
        ))
        .show(move |restart| {
            if restart {
                crate::shutdown(&handle, Duration::from_millis(500));
                handle.restart();
            }
        });
}

/// Send a command to the acquisition thread and wait for its reply.
fn request<T>(
    app: &AppHandle,
    make: impl FnOnce(Reply<T>) -> AcqCommand,
    timeout: Duration,
) -> Result<T, AcqError> {
    let (reply, done) = crossbeam_channel::bounded(1);
    let state = app.state::<AppState>();
    state
        .acq_tx
        .send(make(reply))
        .map_err(|e| AcqError::Bus(e.to_string()))?;
    done.recv_timeout(timeout)
        .map_err(|_| AcqError::Bus("no response from the measurement thread".into()))?
}

/// Blink the bridge LED; returns when the blinking has finished.
pub fn identify(app: &AppHandle) -> Result<(), AcqError> {
    request(
        app,
        |r| AcqCommand::Identify(Some(r)),
        Duration::from_secs(5),
    )
}

/// Switch the Relay Module to `true` or `false`; returns its new state.
pub fn set_relay(app: &AppHandle, state: bool) -> Result<RelayInfo, AcqError> {
    request(
        app,
        |r| AcqCommand::SetRelay(state, Some(r)),
        Duration::from_secs(5),
    )
}

/// Timed relay pulse; the length defaults to the setting. Returns once the
/// relay has switched over.
pub fn pulse_relay(
    app: &AppHandle,
    state: Option<bool>,
    duration: Option<Duration>,
) -> Result<RelayInfo, AcqError> {
    let duration = duration.unwrap_or_else(|| {
        Duration::from_millis(app.state::<AppState>().settings().relay.pulse_duration_ms)
    });
    request(
        app,
        |r| AcqCommand::PulseRelay {
            state,
            duration,
            reply: Some(r),
        },
        Duration::from_secs(5),
    )
}

/// Carry out relay commands received over MQTT, when allowed.
pub fn spawn_mqtt_commands(app: AppHandle, rx: Receiver<MqttCommand>) -> std::io::Result<()> {
    std::thread::Builder::new()
        .name("mqtt-commands".into())
        .spawn(move || {
            for cmd in rx {
                let mqtt = app.state::<AppState>().settings().mqtt.clone();
                if !(mqtt.enabled && mqtt.allow_control) {
                    warn!("ignored MQTT relay command: relay control over MQTT is off");
                    continue;
                }
                let result = match cmd {
                    MqttCommand::SetRelay(p) => set_relay(&app, p),
                    MqttCommand::PulseRelay { state, duration } => {
                        pulse_relay(&app, state, duration)
                    }
                };
                if let Err(e) = result {
                    warn!("MQTT relay command failed: {e}");
                }
            }
        })
        .map(|_| ())
}

/// Detect the Climate Module again and report what was found.
pub fn rescan(app: &AppHandle) -> Result<RescanOutcome, AcqError> {
    request(
        app,
        |r| AcqCommand::Rescan(Some(r)),
        Duration::from_secs(10),
    )
}
