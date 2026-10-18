//! TOWER GPS Module: a u-blox SAM-M8Q on I2C (0x42), its supply, backup
//! supply and LEDs on a TCA9534 (0x21).
//!
//! twr-sdk reads the NMEA stream the receiver sends on I2C by default. Over
//! the FT260 every transfer costs about 25 ms, so here the receiver is told
//! to send only UBX on I2C, and one UBX-NAV-PVT (fix, position, altitude,
//! accuracy, satellites and UTC time in 100 bytes) is polled every few
//! seconds. The backup supply stays on, so the receiver keeps its almanac
//! and finds a fix faster after a restart. The green LED shows a fix.

use std::time::Duration;

use serde::{Deserialize, Serialize};
use tracing::{debug, info};

use crate::bus::{BusError, Clock, I2cBus};

pub const EXPANDER: u8 = 0x21;
pub const RECEIVER: u8 = 0x42;

const REG_OUTPUT: u8 = 0x01;
const REG_CONFIG: u8 = 0x03;
/// Receiver supply (P0), red LED (P3), green LED (P5), backup supply (P7).
const POWER: u8 = 0x01;
/// Only those four pins are outputs until the receiver answers: the
/// expander may belong to something else. P1, P2, P4 and P6 are not
/// connected on the module (schematic rev 1.2); once the receiver answered
/// they are outputs too, rather than floating inputs.
const OUTPUTS: u8 = !(0x01 | 0x08 | 0x20 | 0x80);
const LED_GREEN: u8 = 0x20;
const BACKUP: u8 = 0x80;

/// Bytes waiting (2 bytes, big endian), then the data stream.
const DDC_AVAILABLE: u8 = 0xFD;
const DDC_STREAM: u8 = 0xFF;
const CHUNK: usize = 60;

/// How often the position is polled.
pub const PERIOD: Duration = Duration::from_secs(5);
/// From power on to the receiver taking commands.
const BOOT: Duration = Duration::from_secs(1);
const REPLY_TIME: Duration = Duration::from_millis(100);
const GIVE_UP: u8 = 3;

/// UBX frame: sync, class, id, length (little endian), payload, Fletcher
/// checksum over class to payload.
pub(crate) fn ubx(class: u8, id: u8, payload: &[u8]) -> Vec<u8> {
    let len = u16::try_from(payload.len()).unwrap_or(0).to_le_bytes();
    let mut frame = vec![0xB5, 0x62, class, id, len[0], len[1]];
    frame.extend_from_slice(payload);
    let (mut a, mut b) = (0u8, 0u8);
    for &byte in &frame[2..] {
        a = a.wrapping_add(byte);
        b = b.wrapping_add(a);
    }
    frame.extend_from_slice(&[a, b]);
    frame
}

/// CFG-PRT for the I2C port: slave address 0x42, all protocols in, UBX out.
fn config_port() -> Vec<u8> {
    ubx(
        0x06,
        0x00,
        &[
            0,
            0,
            0,
            0,
            RECEIVER << 1,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0x07,
            0,
            0x01,
            0,
            0,
            0,
            0,
            0,
        ],
    )
}

fn poll_pvt() -> Vec<u8> {
    ubx(0x01, 0x07, &[])
}

/// What the receiver knows.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct GpsInfo {
    pub channel: u8,
    /// A valid 2D or 3D position.
    pub fix: bool,
    /// Satellites used.
    pub satellites: u8,
    /// Degrees (WGS 84); absent without a fix.
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    /// Above mean sea level, in metres (3D fix only).
    pub altitude: Option<f64>,
    /// Estimated horizontal accuracy, in metres.
    pub accuracy: Option<f64>,
    /// UTC time from the satellites (Unix ms), once known.
    pub time: Option<i64>,
}

/// The parts of a UBX-NAV-PVT payload used here.
#[derive(Clone, Debug, PartialEq)]
pub struct NavPvt {
    pub fix_type: u8,
    pub fix_ok: bool,
    pub satellites: u8,
    pub latitude: f64,
    pub longitude: f64,
    pub altitude_msl: f64,
    pub h_accuracy: f64,
    pub time: Option<i64>,
}

impl NavPvt {
    pub fn parse(p: &[u8]) -> Option<Self> {
        if p.len() < 92 {
            return None;
        }
        let i4 = |o: usize| i32::from_le_bytes([p[o], p[o + 1], p[o + 2], p[o + 3]]);
        let u4 = |o: usize| u32::from_le_bytes([p[o], p[o + 1], p[o + 2], p[o + 3]]);
        let year = u16::from_le_bytes([p[4], p[5]]);
        // Valid date and time, fully resolved.
        let time = (p[11] & 0x07 == 0x07).then(|| {
            let days = days_from_civil(i64::from(year), i64::from(p[6]), i64::from(p[7]));
            ((days * 24 + i64::from(p[8])) * 60 + i64::from(p[9])) * 60_000
                + i64::from(p[10]) * 1000
        });
        Some(Self {
            fix_type: p[20],
            fix_ok: p[21] & 0x01 != 0,
            satellites: p[23],
            longitude: f64::from(i4(24)) * 1e-7,
            latitude: f64::from(i4(28)) * 1e-7,
            altitude_msl: f64::from(i4(36)) / 1000.0,
            h_accuracy: f64::from(u4(40)) / 1000.0,
            time,
        })
    }
}

/// Days since 1970-01-01 of a proleptic Gregorian date (H. Hinnant's
/// algorithm).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (m + if m > 2 { -3 } else { 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// The NAV-PVT payloads in `data` (other frames and noise are skipped).
fn find_pvt(data: &[u8]) -> Option<NavPvt> {
    let mut found = None;
    let mut i = 0;
    while i + 8 <= data.len() {
        if data[i] != 0xB5 || data[i + 1] != 0x62 {
            i += 1;
            continue;
        }
        let len = usize::from(u16::from_le_bytes([data[i + 4], data[i + 5]]));
        let end = i + 6 + len + 2;
        if end > data.len() {
            break;
        }
        let frame = &data[i..end];
        if ubx(frame[2], frame[3], &frame[6..6 + len]) == frame
            && frame[2] == 0x01
            && frame[3] == 0x07
        {
            found = NavPvt::parse(&frame[6..6 + len]);
        }
        i = end;
    }
    found
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    /// Powered, waiting for the receiver to boot.
    Booting,
    Running,
}

pub struct GpsModule {
    channel: u8,
    phase: Phase,
    /// Clock time of the next step.
    next: Duration,
    led: bool,
    /// The receiver answered: this is a GPS Module.
    confirmed: bool,
    failures: u8,
    info: Option<GpsInfo>,
}

impl GpsModule {
    /// Whether the module's expander answers on the selected channel.
    pub fn probe(bus: &mut dyn I2cBus) -> Result<bool, BusError> {
        let mut config = [0u8];
        match bus.write_read(EXPANDER, &[REG_CONFIG], &mut config) {
            Ok(()) => Ok(true),
            Err(e) if e.is_fatal() => Err(e),
            Err(_) => Ok(false),
        }
    }

    /// Power the receiver of a module found on `channel` (selected).
    pub fn start(bus: &mut dyn I2cBus, clock: &dyn Clock, channel: u8) -> Result<Self, BusError> {
        bus.write(EXPANDER, &[REG_OUTPUT, BACKUP | POWER])?;
        bus.write(EXPANDER, &[REG_CONFIG, OUTPUTS])?;
        Ok(Self {
            channel,
            phase: Phase::Booting,
            next: clock.now() + BOOT,
            led: false,
            confirmed: false,
            failures: 0,
            info: None,
        })
    }

    pub fn channel(&self) -> u8 {
        self.channel
    }

    pub fn next(&self) -> Duration {
        self.next
    }

    pub fn due(&self, now: Duration) -> bool {
        now >= self.next
    }

    pub fn gone(&self) -> bool {
        self.failures >= GIVE_UP
    }

    /// A step that could not be done (the channel could not be selected):
    /// counts as a failure, and the next try is a period later.
    pub fn missed(&mut self, now: Duration) {
        self.failures = self.failures.saturating_add(1);
        self.next = now + PERIOD;
    }

    pub fn info(&self) -> Option<&GpsInfo> {
        self.info.as_ref()
    }

    /// Do the next step when due (the channel must be selected); returns
    /// whether what the receiver knows changed.
    pub fn service(&mut self, bus: &mut dyn I2cBus, clock: &dyn Clock) -> Result<bool, BusError> {
        if !self.due(clock.now()) {
            return Ok(false);
        }
        let result = match self.phase {
            Phase::Booting => self.configure(bus, clock).map(|()| false),
            Phase::Running => self.poll(bus, clock),
        };
        self.next = clock.now() + PERIOD;
        match result {
            Ok(changed) => {
                self.failures = 0;
                Ok(changed)
            }
            Err(e) if e.is_fatal() => Err(e),
            Err(e) => {
                self.failures = self.failures.saturating_add(1);
                debug!("GPS on channel {} failed: {e}", self.channel);
                Ok(false)
            }
        }
    }

    fn configure(&mut self, bus: &mut dyn I2cBus, clock: &dyn Clock) -> Result<(), BusError> {
        bus.write(RECEIVER, &config_port())?;
        clock.sleep(REPLY_TIME);
        // Whatever it sent so far (NMEA, the acknowledgement) is not needed.
        self.drain(bus)?;
        self.phase = Phase::Running;
        info!("GPS receiver on channel {} ready", self.channel);
        Ok(())
    }

    /// Everything waiting in the receiver's buffer.
    fn drain(&self, bus: &mut dyn I2cBus) -> Result<Vec<u8>, BusError> {
        let mut available = [0u8; 2];
        bus.write_read(RECEIVER, &[DDC_AVAILABLE], &mut available)?;
        let mut left = usize::from(u16::from_be_bytes(available));
        // 0xFFFF: no data (some firmware); cap what one poll reads.
        if left == 0xFFFF {
            left = 0;
        }
        left = left.min(4096);
        let mut data = Vec::with_capacity(left);
        while left > 0 {
            let n = left.min(CHUNK);
            let mut buf = vec![0u8; n];
            bus.write_read(RECEIVER, &[DDC_STREAM], &mut buf)?;
            data.extend_from_slice(&buf);
            left -= n;
        }
        Ok(data)
    }

    fn poll(&mut self, bus: &mut dyn I2cBus, clock: &dyn Clock) -> Result<bool, BusError> {
        self.drain(bus)?;
        bus.write(RECEIVER, &poll_pvt())?;
        clock.sleep(REPLY_TIME);
        let Some(pvt) = find_pvt(&self.drain(bus)?) else {
            return Ok(false);
        };
        if !self.confirmed {
            bus.write(EXPANDER, &[REG_CONFIG, 0x00])?;
            self.confirmed = true;
        }
        let fix = pvt.fix_ok && matches!(pvt.fix_type, 2..=4);
        let info = GpsInfo {
            channel: self.channel,
            fix,
            satellites: pvt.satellites,
            latitude: fix.then_some(pvt.latitude),
            longitude: fix.then_some(pvt.longitude),
            altitude: (fix && pvt.fix_type != 2).then_some(pvt.altitude_msl),
            accuracy: fix.then_some(pvt.h_accuracy),
            time: pvt.time,
        };
        if fix != self.led {
            let led = if fix { LED_GREEN } else { 0 };
            bus.write(EXPANDER, &[REG_OUTPUT, BACKUP | POWER | led])?;
            self.led = fix;
        }
        let changed = self.info.as_ref() != Some(&info);
        self.info = Some(info);
        Ok(changed)
    }

    /// Give the expander's pins back (all inputs): what answered was not a
    /// GPS Module after all.
    pub fn release(&self, bus: &mut dyn I2cBus) -> Result<(), BusError> {
        crate::climate::select_channel(bus, self.channel)?;
        bus.write(EXPANDER, &[REG_CONFIG, 0xFF])
    }

    /// Switch the receiver and the LED off; the backup supply stays on.
    pub fn stop(&self, bus: &mut dyn I2cBus) -> Result<(), BusError> {
        crate::climate::select_channel(bus, self.channel)?;
        bus.write(EXPANDER, &[REG_OUTPUT, BACKUP])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_carry_the_checksum() {
        assert_eq!(
            poll_pvt(),
            vec![0xB5, 0x62, 0x01, 0x07, 0x00, 0x00, 0x08, 0x19]
        );
        // twr-sdk's "enable PUBX" frame, rebuilt.
        assert_eq!(
            ubx(
                0x06,
                0x01,
                &[0xF1, 0x00, 0x01, 0x01, 0x00, 0x01, 0x01, 0x00]
            ),
            vec![
                0xB5, 0x62, 0x06, 0x01, 0x08, 0x00, 0xF1, 0x00, 0x01, 0x01, 0x00, 0x01, 0x01, 0x00,
                0x04, 0x3B
            ]
        );
    }

    fn pvt_payload() -> Vec<u8> {
        let mut p = vec![0u8; 92];
        p[4..6].copy_from_slice(&2026u16.to_le_bytes());
        p[6..11].copy_from_slice(&[10, 17, 19, 30, 5]);
        p[11] = 0x07;
        p[20] = 3;
        p[21] = 0x01;
        p[23] = 9;
        p[24..28].copy_from_slice(&150_563_000i32.to_le_bytes());
        p[28..32].copy_from_slice(&507_712_000i32.to_le_bytes());
        p[36..40].copy_from_slice(&374_500i32.to_le_bytes());
        p[40..44].copy_from_slice(&2_400u32.to_le_bytes());
        p
    }

    #[test]
    fn finds_and_decodes_nav_pvt() {
        let mut data = b"$GNRMC,,V,,,,,,,,,,N*4D\r\n".to_vec();
        data.extend(ubx(0x05, 0x01, &[0x06, 0x00]));
        data.extend(ubx(0x01, 0x07, &pvt_payload()));
        let Some(pvt) = find_pvt(&data) else {
            panic!("no NAV-PVT");
        };
        assert_eq!((pvt.fix_type, pvt.fix_ok, pvt.satellites), (3, true, 9));
        assert!((pvt.latitude - 50.7712).abs() < 1e-9 && (pvt.longitude - 15.0563).abs() < 1e-9);
        assert!((pvt.altitude_msl - 374.5).abs() < 1e-9 && (pvt.h_accuracy - 2.4).abs() < 1e-9);
        assert_eq!(pvt.time, Some(1_792_265_405_000));
    }

    #[test]
    fn a_bad_checksum_is_skipped() {
        let mut frame = ubx(0x01, 0x07, &pvt_payload());
        let last = frame.len() - 1;
        frame[last] ^= 0xFF;
        assert_eq!(find_pvt(&frame), None);
    }

    #[test]
    fn configures_then_polls_and_lights_the_led_with_a_fix() -> Result<(), BusError> {
        use crate::bus::mock::{MockBus, MockClock, attach_gps_module};
        let mut bus = MockBus::new();
        let receiver = attach_gps_module(&mut bus, 1);
        let clock = MockClock::new();
        crate::climate::select_channel(&mut bus, 1)?;
        assert!(GpsModule::probe(&mut bus)?);
        let mut gps = GpsModule::start(&mut bus, &clock, 1)?;
        let led = |bus: &mut MockBus| -> Result<bool, BusError> {
            let mut out = [0u8];
            bus.write_read(EXPANDER, &[REG_OUTPUT], &mut out)?;
            Ok(out[0] & LED_GREEN != 0)
        };

        // Booting, then the port is switched to UBX: no news yet.
        assert!(!gps.service(&mut bus, &clock)?, "not due before boot");
        clock.sleep(BOOT);
        assert!(!gps.service(&mut bus, &clock)?);
        let received = || {
            receiver
                .lock()
                .map(|r| r.received.clone())
                .unwrap_or_default()
        };
        assert_eq!(received(), vec![(0x06, 0x00)]);

        // Searching: satellites but no position.
        if let Ok(mut r) = receiver.lock() {
            r.satellites = 3;
        }
        clock.sleep(PERIOD);
        assert!(gps.service(&mut bus, &clock)?);
        let info = gps.info().cloned();
        assert_eq!(
            info.as_ref().map(|i| (i.fix, i.satellites, i.latitude)),
            Some((false, 3, None))
        );
        assert!(!led(&mut bus)?);

        // A fix: the position, and the green LED as on the module.
        if let Ok(mut r) = receiver.lock() {
            r.position = Some((50.771168, 15.056328, 374.5));
            r.satellites = 9;
        }
        clock.sleep(PERIOD);
        assert!(gps.service(&mut bus, &clock)?);
        let Some(info) = gps.info().cloned() else {
            panic!("no info");
        };
        assert!(info.fix);
        assert!((info.latitude.unwrap_or_default() - 50.771168).abs() < 1e-6);
        assert!((info.longitude.unwrap_or_default() - 15.056328).abs() < 1e-6);
        assert_eq!(info.altitude, Some(374.5));
        assert_eq!(info.accuracy, Some(2.5));
        assert!(led(&mut bus)?);
        assert_eq!(received().iter().filter(|r| **r == (0x01, 0x07)).count(), 2);
        Ok(())
    }
}
