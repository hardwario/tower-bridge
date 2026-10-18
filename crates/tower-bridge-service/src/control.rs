//! What front ends, the API and MQTT ask of the service: settings changes and
//! commands to the bridge. The acquisition thread owns the bridge, so each
//! command goes to it over a channel and waits for its reply.

use std::sync::Arc;
use std::time::Duration;

use crossbeam_channel::Receiver;
use tower_bridge_core::acquisition::{AcqCommand, AcqError, Reply, RescanOutcome};
use tower_bridge_core::mqtt::{MqttCommand, warn_rarely};
use tower_bridge_core::nfc::NfcInfo;
use tower_bridge_core::relay::RelayInfo;
use tracing::warn;

use crate::hub::HubMsg;
use crate::settings::Settings;
use crate::state::ServiceState;

impl ServiceState {
    /// Validate, persist and apply new settings.
    pub fn apply_settings(&self, new: Settings) -> std::io::Result<Arc<Settings>> {
        let _guard = self.settings_lock.lock();
        self.apply_locked(new)
    }

    /// Change part of the current settings and apply them.
    pub fn update_settings(
        &self,
        change: impl FnOnce(&mut Settings),
    ) -> std::io::Result<Arc<Settings>> {
        let _guard = self.settings_lock.lock();
        let mut s = (*self.settings()).clone();
        change(&mut s);
        self.apply_locked(s)
    }

    fn apply_locked(&self, new: Settings) -> std::io::Result<Arc<Settings>> {
        let new = Arc::new(new.validated());
        new.save(&self.paths.settings_file())?;
        let old = std::mem::replace(&mut *self.settings.write(), new.clone());
        if old.sample_interval_ms != new.sample_interval_ms {
            let _ = self
                .acq_tx
                .send(AcqCommand::SetInterval(Duration::from_millis(
                    new.sample_interval_ms,
                )));
        }
        if old.i2c_channel != new.i2c_channel {
            let _ = self.acq_tx.send(AcqCommand::SetChannel(new.i2c_channel));
        }
        let _ = self.hub_tx.send(HubMsg::SettingsChanged {
            old: old.clone(),
            new: new.clone(),
        });
        if old.api != new.api {
            self.apply_api();
        }
        self.observer().settings_changed(self, &old, &new);
        Ok(new)
    }

    /// Send a command to the acquisition thread and wait for its reply.
    fn request<T>(
        &self,
        make: impl FnOnce(Reply<T>) -> AcqCommand,
        timeout: Duration,
    ) -> Result<T, AcqError> {
        let (reply, done) = crossbeam_channel::bounded(1);
        self.acq_tx
            .send(make(reply))
            .map_err(|e| AcqError::Bus(e.to_string()))?;
        done.recv_timeout(timeout)
            .map_err(|_| AcqError::Bus("no response from the measurement thread".into()))?
    }

    /// Blink the bridge LED; returns when the blinking has finished.
    pub fn identify(&self) -> Result<(), AcqError> {
        self.request(|r| AcqCommand::Identify(Some(r)), Duration::from_secs(5))
    }

    /// Look for the sensors again and report what was found.
    pub fn rescan(&self) -> Result<RescanOutcome, AcqError> {
        self.request(|r| AcqCommand::Rescan(Some(r)), Duration::from_secs(10))
    }

    /// Switch the Relay Module to `true` or `false`; returns its new state.
    pub fn set_relay(&self, state: bool) -> Result<RelayInfo, AcqError> {
        self.request(
            |r| AcqCommand::SetRelay(state, Some(r)),
            Duration::from_secs(5),
        )
    }

    /// Timed relay pulse; the length defaults to the setting. Returns once
    /// the relay has switched over.
    pub fn pulse_relay(
        &self,
        state: Option<bool>,
        duration: Option<Duration>,
    ) -> Result<RelayInfo, AcqError> {
        let duration = duration
            .unwrap_or_else(|| Duration::from_millis(self.settings().relay.pulse_duration_ms));
        self.request(
            |r| AcqCommand::PulseRelay {
                state,
                duration,
                reply: Some(r),
            },
            Duration::from_secs(5),
        )
    }

    /// Write a URL to the NFC Tag; returns what the tag holds afterwards. Up
    /// to 240 bytes take 16 blocks of about 25 ms each, after the
    /// measurement that may be running.
    pub fn write_nfc_url(&self, url: String) -> Result<NfcInfo, AcqError> {
        self.request(
            |r| AcqCommand::WriteNfcUrl(url, Some(r)),
            Duration::from_secs(10),
        )
    }
}

/// Carry out relay commands received over MQTT, when allowed.
pub(crate) fn spawn_mqtt_commands(
    state: Arc<ServiceState>,
    rx: Receiver<MqttCommand>,
) -> std::io::Result<()> {
    std::thread::Builder::new()
        .name("mqtt-commands".into())
        .spawn(move || {
            while let Ok(mut cmd) = rx.recv() {
                // A burst (a looping automation) switches once: the last
                // command waiting wins, as with the other ways in.
                while let Ok(next) = rx.try_recv() {
                    cmd = next;
                }
                let mqtt = state.settings().mqtt.clone();
                if !(mqtt.enabled && mqtt.allow_control) {
                    warn_rarely("ignored MQTT relay command: relay control over MQTT is off");
                    continue;
                }
                let relay_channel = state.relay.read().map(|r| r.channel);
                if relay_channel != Some(cmd.channel()) {
                    warn_rarely(&format!(
                        "ignored MQTT relay command: no Relay Module on I2C channel {}",
                        cmd.channel()
                    ));
                    continue;
                }
                let result = match cmd {
                    MqttCommand::SetRelay { state: on, .. } => state.set_relay(on),
                    MqttCommand::PulseRelay {
                        state: on,
                        duration,
                        ..
                    } => state.pulse_relay(on, duration),
                };
                if let Err(e) = result {
                    warn!("MQTT relay command failed: {e}");
                }
            }
        })
        .map(|_| ())
}
