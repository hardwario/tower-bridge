//! TOWER Relay Module: a latching (bistable) relay switched by an H-bridge on
//! a TCA9534A I/O expander.
//!
//! Modules are fitted with either a TCA9534A or a TCA9534; both have the same
//! registers but different base addresses.
//!
//! Expander pins (schematic rev 1.5): P0 green LED, P1 red LED, P4 RELAY_F_H,
//! P5 RELAY_T_L, P6 RELAY_T_H, P7 RELAY_F_L. The high-side transistors are
//! P-channel (on when their pin is low), the low-side ones N-channel (on when
//! high). A short coil pulse in one direction moves the contacts to one side,
//! the other direction to the other side; the coil is then de-energised and
//! the contacts stay put, even without power.
//!
//! The relay's state is `true` (the "T" direction, green LED, as twr-sdk
//! `true`) or `false` (the "F" direction, red LED); checked on hardware. An
//! unknown state is `None` (JSON `null`).

use std::time::Duration;

use serde::{Deserialize, Serialize};
use tracing::{debug, info};

use crate::bus::{BusError, Clock, I2cBus};
use crate::climate::select_channel;

/// TCA9534A address with the default strapping (A2 low, A1 and A0 high).
pub const ADDR_DEFAULT: u8 = 0x3B;
/// TCA9534A address with R3 fitted and R6 removed (A2 high).
pub const ADDR_ALTERNATE: u8 = 0x3F;
/// The same strappings on a TCA9534 (base address 0x20 instead of 0x38).
pub const ADDR_TCA9534_DEFAULT: u8 = 0x23;
pub const ADDR_TCA9534_ALTERNATE: u8 = 0x27;
/// Probe order.
pub const ADDRESSES: [u8; 4] = [
    ADDR_DEFAULT,
    ADDR_ALTERNATE,
    ADDR_TCA9534_DEFAULT,
    ADDR_TCA9534_ALTERNATE,
];

const REG_OUTPUT: u8 = 0x01;
const REG_CONFIG: u8 = 0x03;

const LED_GREEN: u8 = 1 << 0;
const LED_RED: u8 = 1 << 1;
/// Coil current in the T direction: RELAY_T_H (P6) low, RELAY_T_L (P5) high.
const DRIVE_T: u8 = (1 << 4) | (1 << 5);
/// Coil current in the F direction: RELAY_F_H (P4) low, RELAY_F_L (P7) high.
const DRIVE_F: u8 = (1 << 6) | (1 << 7);
/// All four transistors off: high sides held high, low sides low.
const IDLE: u8 = (1 << 4) | (1 << 6);

/// Coil pulse length (the bridge adds a few milliseconds of transfer time).
pub const COIL_PULSE: Duration = Duration::from_millis(20);
/// Minimum time between coil pulses, so the reservoir capacitor can recharge.
pub const RECHARGE: Duration = Duration::from_millis(300);
/// Shortest and longest timed pulse (switch over, then back).
pub const MIN_PULSE: Duration = Duration::from_millis(500);
pub const MAX_PULSE: Duration = Duration::from_secs(3600);
/// Timed pulse length unless configured otherwise.
pub const DEFAULT_PULSE: Duration = Duration::from_secs(2);

/// A pulse length given in milliseconds (API, MQTT); `None` unless positive.
/// Longer values are cut to [`MAX_PULSE`].
pub fn pulse_from_ms(ms: f64) -> Option<Duration> {
    (ms.is_finite() && ms > 0.0)
        .then(|| Duration::from_millis(ms.min(MAX_PULSE.as_millis() as f64) as u64))
}

/// Case-insensitive "true" / "false".
pub fn parse_state(s: &str) -> Option<bool> {
    match s.trim().to_ascii_lowercase().as_str() {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

fn drive(state: bool) -> u8 {
    if state { DRIVE_T } else { DRIVE_F }
}

fn state_led(state: bool) -> u8 {
    if state { LED_GREEN } else { LED_RED }
}

/// A detected Relay Module.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct RelayInfo {
    /// I2C switch channel on the Bridge Module.
    pub channel: u8,
    pub address: u8,
    /// `true`, `false` or unknown: a latching relay cannot be
    /// read back, so the state is known only after it has been switched (the
    /// expander remembers it while it stays powered).
    pub state: Option<bool>,
    /// While a timed pulse runs: when it ends (Unix ms) and the relay
    /// switches back.
    pub pulse_until: Option<i64>,
}

fn led(state: Option<bool>) -> u8 {
    state.map_or(0, state_led)
}

/// State recorded in the LED pins by an earlier switch. After power-up the
/// expander's output register reads 0xFF (both LEDs), which means unknown.
fn state_from_output(out: u8) -> Option<bool> {
    match (out & LED_GREEN != 0, out & LED_RED != 0) {
        (true, false) => Some(true),
        (false, true) => Some(false),
        _ => None,
    }
}

/// Non-fatal errors mean "not here"; transport failures propagate.
fn absent_on_nack<T>(r: Result<T, BusError>) -> Result<Option<T>, BusError> {
    match r {
        Ok(v) => Ok(Some(v)),
        Err(e) if e.is_fatal() => Err(e),
        Err(e) => {
            debug!("relay probe: {e}");
            Ok(None)
        }
    }
}

pub struct RelayModule {
    channel: u8,
    address: u8,
    state: Option<bool>,
    last_pulse: Option<Duration>,
}

impl RelayModule {
    /// Look for a Relay Module on the given switch channels and make its pins
    /// outputs without moving the relay. `Ok(None)` when nothing answers.
    pub fn detect(bus: &mut dyn I2cBus, channels: &[u8]) -> Result<Option<Self>, BusError> {
        for &channel in channels {
            if absent_on_nack(select_channel(bus, channel))?.is_none() {
                continue;
            }
            for address in ADDRESSES {
                let mut out = [0u8];
                if absent_on_nack(bus.write_read(address, &[REG_OUTPUT], &mut out))?.is_none() {
                    continue;
                }
                let relay = Self {
                    channel,
                    address,
                    state: state_from_output(out[0]),
                    last_pulse: None,
                };
                if absent_on_nack(relay.configure(bus))?.is_none() {
                    continue;
                }
                info!("Relay Module found on I2C channel {channel} at 0x{address:02x}");
                return Ok(Some(relay));
            }
        }
        Ok(None)
    }

    pub fn state(&self) -> Option<bool> {
        self.state
    }

    pub fn info(&self) -> RelayInfo {
        RelayInfo {
            channel: self.channel,
            address: self.address,
            state: self.state,
            pulse_until: None,
        }
    }

    /// Idle drive (keeping the LED), then all pins as outputs. Repeated before
    /// every switch in case the module lost power in between.
    fn configure(&self, bus: &mut dyn I2cBus) -> Result<(), BusError> {
        bus.write(self.address, &[REG_OUTPUT, IDLE | led(self.state)])?;
        bus.write(self.address, &[REG_CONFIG, 0x00])
    }

    /// Switch to `state` with one coil pulse.
    pub fn set(
        &mut self,
        bus: &mut dyn I2cBus,
        clock: &dyn Clock,
        state: bool,
    ) -> Result<(), BusError> {
        if let Some(t) = self.last_pulse {
            clock.sleep_until(t + RECHARGE);
        }
        let a = self.address;
        select_channel(bus, self.channel)?;
        self.configure(bus)?;
        if let Err(e) = bus.write(a, &[REG_OUTPUT, drive(state) | state_led(state)]) {
            // The expander may have latched the drive before the error came:
            // release the coil before giving up.
            if !e.is_fatal() {
                let idle = [REG_OUTPUT, IDLE | led(self.state)];
                if bus.write(a, &idle).is_err() {
                    let _ = bus.recover();
                    let _ = bus.write(a, &idle);
                }
            }
            return Err(e);
        }
        clock.sleep(COIL_PULSE);
        let idle = [REG_OUTPUT, IDLE | state_led(state)];
        let released = bus.write(a, &idle);
        self.last_pulse = Some(clock.now());
        if let Err(e) = released {
            // Never leave the coil energised: recover the bus and try again.
            if e.is_fatal() {
                return Err(e);
            }
            bus.recover()?;
            bus.write(a, &idle)?;
        }
        self.state = Some(state);
        info!("relay switched {state}");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::mock::{MockBus, MockClock, SimTca9534a, Transaction};

    fn bus_with_relay(channel: usize, address: u8) -> MockBus {
        let mut bus = MockBus::new();
        bus.attach(channel, address, Box::new(SimTca9534a::new()));
        bus
    }

    #[test]
    fn detects_on_either_channel_and_address() -> Result<(), BusError> {
        let mut bus = bus_with_relay(1, ADDR_ALTERNATE);
        let relay = RelayModule::detect(&mut bus, &[0, 1])?;
        assert_eq!(
            relay.map(|r| r.info()),
            Some(RelayInfo {
                channel: 1,
                address: ADDR_ALTERNATE,
                state: None,
                pulse_until: None,
            })
        );
        Ok(())
    }

    #[test]
    fn detects_a_tca9534_without_a() -> Result<(), BusError> {
        let mut bus = bus_with_relay(0, ADDR_TCA9534_DEFAULT);
        let relay = RelayModule::detect(&mut bus, &[0, 1])?;
        assert_eq!(relay.map(|r| r.info().address), Some(ADDR_TCA9534_DEFAULT));
        Ok(())
    }

    #[test]
    fn absent_module() -> Result<(), BusError> {
        let mut bus = MockBus::new();
        assert!(RelayModule::detect(&mut bus, &[0, 1])?.is_none());
        Ok(())
    }

    #[test]
    fn detection_configures_outputs_without_switching() -> Result<(), BusError> {
        let mut bus = bus_with_relay(0, ADDR_DEFAULT);
        RelayModule::detect(&mut bus, &[0])?;
        let writes: Vec<_> = bus
            .log
            .iter()
            .filter(|t| matches!(t, Transaction::Write(a, _) if *a == ADDR_DEFAULT))
            .cloned()
            .collect();
        assert_eq!(
            writes,
            vec![
                Transaction::Write(ADDR_DEFAULT, vec![REG_OUTPUT, IDLE]),
                Transaction::Write(ADDR_DEFAULT, vec![REG_CONFIG, 0x00]),
            ]
        );
        Ok(())
    }

    #[test]
    fn switching_pulses_the_coil_and_releases_it() -> Result<(), BusError> {
        let mut bus = bus_with_relay(0, ADDR_DEFAULT);
        let clock = MockClock::new();
        let Some(mut relay) = RelayModule::detect(&mut bus, &[0])? else {
            panic!("relay not detected");
        };
        bus.log.clear();
        relay.set(&mut bus, &clock, false)?;
        assert_eq!(
            bus.log,
            vec![
                Transaction::Write(0x70, vec![0x01]),
                Transaction::Write(ADDR_DEFAULT, vec![REG_OUTPUT, IDLE]),
                Transaction::Write(ADDR_DEFAULT, vec![REG_CONFIG, 0x00]),
                Transaction::Write(ADDR_DEFAULT, vec![REG_OUTPUT, 0b1100_0010]),
                Transaction::Write(ADDR_DEFAULT, vec![REG_OUTPUT, 0b0101_0010]),
            ]
        );
        assert_eq!(relay.info().state, Some(false));

        bus.log.clear();
        let before = clock.now();
        relay.set(&mut bus, &clock, true)?;
        assert!(clock.now() - before >= RECHARGE, "waits for the capacitor");
        assert_eq!(
            bus.log[3..],
            [
                Transaction::Write(ADDR_DEFAULT, vec![REG_OUTPUT, 0b0011_0001]),
                Transaction::Write(ADDR_DEFAULT, vec![REG_OUTPUT, 0b0101_0001]),
            ]
        );
        assert_eq!(relay.info().state, Some(true));
        Ok(())
    }

    #[test]
    fn state_survives_an_app_restart_via_the_led_pins() -> Result<(), BusError> {
        let mut bus = bus_with_relay(0, ADDR_DEFAULT);
        let clock = MockClock::new();
        if let Some(mut relay) = RelayModule::detect(&mut bus, &[0])? {
            relay.set(&mut bus, &clock, false)?;
        }
        let again = RelayModule::detect(&mut bus, &[0])?;
        assert_eq!(again.map(|r| r.info().state), Some(Some(false)));
        Ok(())
    }

    #[test]
    fn decodes_led_state() {
        assert_eq!(state_from_output(0xFF), None);
        assert_eq!(state_from_output(IDLE), None);
        assert_eq!(state_from_output(IDLE | LED_GREEN), Some(true));
        assert_eq!(state_from_output(IDLE | LED_RED), Some(false));
    }

    #[test]
    fn parses_states() {
        assert_eq!(parse_state(" True "), Some(true));
        assert_eq!(parse_state("FALSE"), Some(false));
        assert_eq!(parse_state("on"), None);
    }
}
