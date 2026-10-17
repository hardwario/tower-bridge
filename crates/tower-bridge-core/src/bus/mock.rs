//! Simulated TOWER Bridge with a Climate Module, used by unit tests and by
//! the app's simulation mode (development without hardware).
//!
//! The simulation models the I2C switch at 0x70 and each sensor's register
//! behaviour closely enough to exercise the real drivers.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::{BusError, Clock, I2cBus};
use crate::model::{Backend, ClimateRevision, DeviceInfo, Values};
use crate::sensors::crc::crc8;

/// One simulated I2C target.
pub trait SimDevice: Send {
    fn write(&mut self, data: &[u8]) -> Result<(), BusError>;
    fn read(&mut self, buf: &mut [u8]) -> Result<(), BusError>;
}

/// Logged bus transaction.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Transaction {
    Write(u8, Vec<u8>),
    Read(u8, usize),
    WriteRead(u8, Vec<u8>, usize),
}

/// Physical environment seen by the simulated sensors.
pub type SharedEnv = Arc<Mutex<Values>>;

const SWITCH_ADDR: u8 = 0x70;

/// Simulated bridge: an I2C switch with two downstream channels.
pub struct MockBus {
    channels: [HashMap<u8, Box<dyn SimDevice>>; 2],
    switch_mask: u8,
    info: DeviceInfo,
    pub log: Vec<Transaction>,
    /// Errors to inject, consumed one per transaction.
    pub inject: Vec<BusError>,
    pub led: bool,
}

impl MockBus {
    pub fn new() -> Self {
        Self {
            channels: [HashMap::new(), HashMap::new()],
            switch_mask: 0,
            info: DeviceInfo {
                backend: Backend::Hid,
                serial: "SIMULATED".into(),
                path: "simulated".into(),
                chip: Some("simulated".into()),
            },
            log: Vec::new(),
            inject: Vec::new(),
            led: false,
        }
    }

    pub fn attach(&mut self, channel: usize, addr: u8, dev: Box<dyn SimDevice>) {
        self.channels[channel].insert(addr, dev);
    }

    pub fn detach_all(&mut self, channel: usize) {
        self.channels[channel].clear();
    }

    /// Bus with a Climate Module of the given revision on `channel`.
    pub fn with_climate(revision: ClimateRevision, channel: usize, env: SharedEnv) -> Self {
        let mut bus = Self::new();
        attach_climate(&mut bus, revision, channel, env);
        bus
    }

    fn device(&mut self, addr: u8) -> Result<&mut Box<dyn SimDevice>, BusError> {
        let mask = self.switch_mask;
        let [ch0, ch1] = &mut self.channels;
        if mask & 0x01 != 0 && ch0.contains_key(&addr) {
            return ch0.get_mut(&addr).ok_or(BusError::AddrNack);
        }
        if mask & 0x02 != 0 {
            return ch1.get_mut(&addr).ok_or(BusError::AddrNack);
        }
        Err(BusError::AddrNack)
    }

    fn injected(&mut self) -> Result<(), BusError> {
        if self.inject.is_empty() {
            Ok(())
        } else {
            Err(self.inject.remove(0))
        }
    }
}

impl Default for MockBus {
    fn default() -> Self {
        Self::new()
    }
}

/// Attach the sensors of a Climate Module revision to a bus channel.
pub fn attach_climate(
    bus: &mut MockBus,
    revision: ClimateRevision,
    channel: usize,
    env: SharedEnv,
) {
    match revision {
        ClimateRevision::R1 => {
            bus.attach(channel, 0x48, Box::new(SimTmp112::new(env.clone())));
            bus.attach(channel, 0x40, Box::new(SimSht20::new(env.clone())));
        }
        ClimateRevision::R2 => {
            bus.attach(channel, 0x45, Box::new(SimSht30::new(env.clone())));
        }
    }
    bus.attach(channel, 0x44, Box::new(SimOpt3001::new(env.clone())));
    bus.attach(channel, 0x60, Box::new(SimMpl3115a2::new(env)));
}

impl I2cBus for MockBus {
    fn write(&mut self, addr: u8, data: &[u8]) -> Result<(), BusError> {
        self.log.push(Transaction::Write(addr, data.to_vec()));
        self.injected()?;
        if addr == SWITCH_ADDR {
            self.switch_mask = data.first().copied().unwrap_or(0) & 0x03;
            return Ok(());
        }
        self.device(addr)?.write(data)
    }

    fn read(&mut self, addr: u8, buf: &mut [u8]) -> Result<(), BusError> {
        self.log.push(Transaction::Read(addr, buf.len()));
        self.injected()?;
        if addr == SWITCH_ADDR {
            buf.fill(self.switch_mask);
            return Ok(());
        }
        self.device(addr)?.read(buf)
    }

    fn write_read(&mut self, addr: u8, wr: &[u8], rd: &mut [u8]) -> Result<(), BusError> {
        self.log
            .push(Transaction::WriteRead(addr, wr.to_vec(), rd.len()));
        self.injected()?;
        let dev = self.device(addr)?;
        dev.write(wr)?;
        dev.read(rd)
    }

    fn recover(&mut self) -> Result<(), BusError> {
        Ok(())
    }

    fn set_led(&mut self, on: bool) -> Result<(), BusError> {
        self.led = on;
        Ok(())
    }

    fn info(&self) -> &DeviceInfo {
        &self.info
    }
}

/// TCA9534A I/O expander (Relay Module): input, output, polarity and
/// configuration registers behind a command (pointer) byte.
pub struct SimTca9534a {
    regs: [u8; 4],
    pointer: usize,
}

impl SimTca9534a {
    pub fn new() -> Self {
        Self {
            regs: [0xFF, 0xFF, 0x00, 0xFF],
            pointer: 0,
        }
    }
}

impl Default for SimTca9534a {
    fn default() -> Self {
        Self::new()
    }
}

impl SimDevice for SimTca9534a {
    fn write(&mut self, data: &[u8]) -> Result<(), BusError> {
        let Some((&cmd, rest)) = data.split_first() else {
            return Ok(());
        };
        if cmd > 3 {
            return Err(BusError::DataNack);
        }
        self.pointer = cmd as usize;
        if let Some(&v) = rest.first()
            && self.pointer != 0
        {
            self.regs[self.pointer] = v;
        }
        Ok(())
    }

    fn read(&mut self, buf: &mut [u8]) -> Result<(), BusError> {
        // Output pins read back what they drive; inputs float high.
        let [_, output, polarity, config] = self.regs;
        let regs = [
            ((output & !config) | config) ^ polarity,
            output,
            polarity,
            config,
        ];
        buf.fill(regs[self.pointer]);
        Ok(())
    }
}

/// Virtual time: `sleep` advances the clock instantly.
#[derive(Default)]
pub struct MockClock {
    now: Mutex<Duration>,
}

impl MockClock {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Clock for MockClock {
    fn now(&self) -> Duration {
        self.now.lock().map(|t| *t).unwrap_or_default()
    }

    fn sleep(&self, d: Duration) {
        if let Ok(mut t) = self.now.lock() {
            *t += d;
        }
    }
}

fn env_value(env: &SharedEnv, f: impl Fn(&Values) -> Option<f64>, default: f64) -> f64 {
    env.lock().ok().and_then(|v| f(&v)).unwrap_or(default)
}

fn copy_out(buf: &mut [u8], src: &[u8]) {
    for (i, b) in buf.iter_mut().enumerate() {
        *b = src.get(i).copied().unwrap_or(0);
    }
}

/// SHT30: NACKs reads unless a measurement was triggered.
pub struct SimSht30 {
    env: SharedEnv,
    pending: bool,
}

impl SimSht30 {
    pub fn new(env: SharedEnv) -> Self {
        Self {
            env,
            pending: false,
        }
    }
}

impl SimDevice for SimSht30 {
    fn write(&mut self, data: &[u8]) -> Result<(), BusError> {
        match data {
            [0x30, 0xA2] => self.pending = false,
            [0x24, 0x00] => self.pending = true,
            _ => return Err(BusError::DataNack),
        }
        Ok(())
    }

    fn read(&mut self, buf: &mut [u8]) -> Result<(), BusError> {
        if !self.pending {
            return Err(BusError::AddrNack);
        }
        self.pending = false;
        let t = env_value(&self.env, |v| v.temperature, 22.0);
        let rh = env_value(&self.env, |v| v.humidity, 45.0);
        let t_raw = (((t + 45.0) / 175.0) * 65535.0).round().clamp(0.0, 65535.0) as u16;
        let rh_raw = ((rh / 100.0) * 65535.0).round().clamp(0.0, 65535.0) as u16;
        let t_b = t_raw.to_be_bytes();
        let rh_b = rh_raw.to_be_bytes();
        let out = [
            t_b[0],
            t_b[1],
            crc8(&t_b, 0xFF),
            rh_b[0],
            rh_b[1],
            crc8(&rh_b, 0xFF),
        ];
        copy_out(buf, &out);
        Ok(())
    }
}

/// SHT20 in no-hold mode.
pub struct SimSht20 {
    env: SharedEnv,
    pending: Option<u8>,
}

impl SimSht20 {
    pub fn new(env: SharedEnv) -> Self {
        Self { env, pending: None }
    }
}

impl SimDevice for SimSht20 {
    fn write(&mut self, data: &[u8]) -> Result<(), BusError> {
        match data {
            [0xFE] => self.pending = None,
            [cmd @ (0xF3 | 0xF5)] => self.pending = Some(*cmd),
            _ => return Err(BusError::DataNack),
        }
        Ok(())
    }

    fn read(&mut self, buf: &mut [u8]) -> Result<(), BusError> {
        let raw = match self.pending.take() {
            Some(0xF3) => {
                let t = env_value(&self.env, |v| v.temperature, 22.0);
                (((t + 46.85) / 175.72) * 65536.0) as u16 & !0x3
            }
            Some(_) => {
                let rh = env_value(&self.env, |v| v.humidity, 45.0);
                ((((rh + 6.0) / 125.0) * 65536.0) as u16 & !0x3) | 0x02
            }
            None => return Err(BusError::AddrNack),
        };
        let b = raw.to_be_bytes();
        copy_out(buf, &[b[0], b[1], crc8(&b, 0x00)]);
        Ok(())
    }
}

/// TMP112 with pointer register and one-shot conversion.
pub struct SimTmp112 {
    env: SharedEnv,
    pointer: u8,
    config: [u8; 2],
}

impl SimTmp112 {
    pub fn new(env: SharedEnv) -> Self {
        Self {
            env,
            pointer: 0,
            config: [0x60, 0xA0],
        }
    }
}

impl SimDevice for SimTmp112 {
    fn write(&mut self, data: &[u8]) -> Result<(), BusError> {
        let (&ptr, rest) = data.split_first().ok_or(BusError::DataNack)?;
        self.pointer = ptr;
        if ptr == 0x01 && rest.len() == 2 {
            // One-shot completes instantly; OS reads back as 1.
            self.config = [rest[0] | 0x80, rest[1]];
        }
        Ok(())
    }

    fn read(&mut self, buf: &mut [u8]) -> Result<(), BusError> {
        match self.pointer {
            0x00 => {
                let t = env_value(&self.env, |v| v.temperature, 22.0);
                let raw = ((t / 0.0625).round() as i16) << 4;
                copy_out(buf, &raw.to_be_bytes());
            }
            0x01 => copy_out(buf, &self.config),
            _ => copy_out(buf, &[0, 0]),
        }
        Ok(())
    }
}

/// OPT3001 with config and result registers.
pub struct SimOpt3001 {
    env: SharedEnv,
    pointer: u8,
    config: [u8; 2],
}

impl SimOpt3001 {
    pub fn new(env: SharedEnv) -> Self {
        Self {
            env,
            pointer: 0,
            config: [0xC8, 0x10],
        }
    }
}

impl SimDevice for SimOpt3001 {
    fn write(&mut self, data: &[u8]) -> Result<(), BusError> {
        let (&ptr, rest) = data.split_first().ok_or(BusError::DataNack)?;
        self.pointer = ptr;
        if ptr == 0x01 && rest.len() == 2 {
            let single_shot = rest[0] & 0x06 == 0x02;
            self.config = if single_shot {
                // Conversion done: back to shutdown, conversion-ready set.
                [rest[0] & !0x06, rest[1] | 0x80]
            } else {
                [rest[0], rest[1] & !0x80]
            };
        }
        Ok(())
    }

    fn read(&mut self, buf: &mut [u8]) -> Result<(), BusError> {
        match self.pointer {
            0x00 => {
                let lux = env_value(&self.env, |v| v.illuminance, 300.0).max(0.0);
                let mut e = 0u16;
                let mut m = lux / 0.01;
                while m > 4095.0 && e < 11 {
                    m /= 2.0;
                    e += 1;
                }
                let raw = (e << 12) | (m.round().min(4095.0) as u16);
                copy_out(buf, &raw.to_be_bytes());
            }
            0x01 => copy_out(buf, &self.config),
            0x7E => copy_out(buf, &[0x54, 0x49]),
            0x7F => copy_out(buf, &[0x30, 0x01]),
            _ => copy_out(buf, &[0, 0]),
        }
        Ok(())
    }
}

/// MPL3115A2 with auto-incrementing register reads.
pub struct SimMpl3115a2 {
    env: SharedEnv,
    pointer: u8,
    ctrl1: u8,
    status: u8,
    out: [u8; 5],
}

impl SimMpl3115a2 {
    pub fn new(env: SharedEnv) -> Self {
        Self {
            env,
            pointer: 0,
            ctrl1: 0,
            status: 0,
            out: [0; 5],
        }
    }
}

impl SimDevice for SimMpl3115a2 {
    fn write(&mut self, data: &[u8]) -> Result<(), BusError> {
        let (&ptr, rest) = data.split_first().ok_or(BusError::DataNack)?;
        self.pointer = ptr;
        if ptr == 0x26
            && let Some(&v) = rest.first()
        {
            if v & 0x04 != 0 {
                self.ctrl1 = 0;
                self.status = 0;
                // The real device does not acknowledge the reset write.
                return Err(BusError::DataNack);
            }
            self.ctrl1 = v & !0x02;
            if v & 0x02 != 0 {
                let p = env_value(&self.env, |v| v.pressure, 101_325.0);
                let raw = ((p * 4.0).round() as u32) << 4;
                let t = env_value(&self.env, |v| v.temperature, 22.0);
                let traw = ((t * 16.0).round() as i16) << 4;
                let tb = traw.to_be_bytes();
                self.out = [(raw >> 16) as u8, (raw >> 8) as u8, raw as u8, tb[0], tb[1]];
                self.status = 0x0E;
            }
        }
        Ok(())
    }

    fn read(&mut self, buf: &mut [u8]) -> Result<(), BusError> {
        for (i, b) in buf.iter_mut().enumerate() {
            let reg = self.pointer.wrapping_add(i as u8);
            *b = match reg {
                0x00 => self.status,
                0x01..=0x05 => self.out[usize::from(reg - 1)],
                0x0C => 0xC4,
                0x26 => self.ctrl1,
                _ => 0,
            };
        }
        if self.pointer == 0x01 {
            self.status = 0;
        }
        Ok(())
    }
}

/// A simulated Climate Module R2 whose environment drifts slowly, for running
/// the app without hardware.
pub struct SimulatedBridge {
    bus: MockBus,
    env: SharedEnv,
    seed: u64,
}

impl SimulatedBridge {
    pub fn new() -> Self {
        let env: SharedEnv = Arc::new(Mutex::new(Values::default()));
        let mut bus = MockBus::with_climate(ClimateRevision::R2, 0, env.clone());
        bus.attach(0, crate::relay::ADDR_DEFAULT, Box::new(SimTca9534a::new()));
        let mut s = Self {
            bus,
            env,
            seed: 0x2545_F491_4F6C_DD1D,
        };
        s.update_env();
        s
    }

    fn noise(&mut self) -> f64 {
        // xorshift64*
        self.seed ^= self.seed >> 12;
        self.seed ^= self.seed << 25;
        self.seed ^= self.seed >> 27;
        let x = self.seed.wrapping_mul(0x2545_F491_4F6C_DD1D);
        (x >> 11) as f64 / (1u64 << 53) as f64 - 0.5
    }

    fn update_env(&mut self) {
        let wall = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs_f64())
            .unwrap_or(0.0);
        let n = [self.noise(), self.noise(), self.noise(), self.noise()];
        let values = simulated_values(wall, n);
        if let Ok(mut env) = self.env.lock() {
            *env = values;
        }
    }
}

/// Plausible indoor climate as a function of Unix time (seconds), with a
/// daily cycle and slow drifts; `noise` components are in -0.5..0.5.
pub fn simulated_values(wall: f64, noise: [f64; 4]) -> Values {
    use std::f64::consts::TAU;
    let local = wall + 7200.0;
    let day = (local % 86_400.0) / 86_400.0 * TAU;
    let drift = (wall / 1800.0).sin();
    let weather = (wall / (86_400.0 * 2.3)).sin() + 0.5 * (wall / (86_400.0 * 0.9)).cos();
    Values {
        temperature: Some(
            22.3 + 2.2 * (day - 2.0).sin() + 0.35 * drift + 0.6 * weather + 0.05 * noise[0],
        ),
        humidity: Some(
            45.0 - 5.0 * (day - 2.0).sin() + 1.5 * drift - 3.0 * weather + 0.2 * noise[1],
        ),
        illuminance: Some(((day - 1.75).sin() * 720.0).max(0.0) + 25.0 + 8.0 * noise[2]),
        pressure: Some(100_150.0 + 650.0 * weather + 40.0 * (wall / 7200.0).sin() + 3.0 * noise[3]),
    }
}

impl Default for SimulatedBridge {
    fn default() -> Self {
        Self::new()
    }
}

impl I2cBus for SimulatedBridge {
    fn write(&mut self, addr: u8, data: &[u8]) -> Result<(), BusError> {
        if addr == SWITCH_ADDR {
            // Start of a measurement cycle.
            self.update_env();
            self.bus.log.clear();
        }
        self.bus.write(addr, data)
    }

    fn read(&mut self, addr: u8, buf: &mut [u8]) -> Result<(), BusError> {
        self.bus.read(addr, buf)
    }

    fn write_read(&mut self, addr: u8, wr: &[u8], rd: &mut [u8]) -> Result<(), BusError> {
        self.bus.write_read(addr, wr, rd)
    }

    fn recover(&mut self) -> Result<(), BusError> {
        Ok(())
    }

    fn set_led(&mut self, on: bool) -> Result<(), BusError> {
        self.bus.set_led(on)
    }

    fn info(&self) -> &DeviceInfo {
        self.bus.info()
    }
}
