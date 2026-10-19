//! Simulated TOWER Bridge with a Climate Module and sensor tags, used by unit
//! tests and by the app's simulation mode (development without hardware).
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
                bridges_connected: 1,
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

/// The TOWER CO2 Module as the driver sees it: its TCA9534A at 0x38 (supply,
/// enable, RDY on P7) and the SC16IS740 UART at 0x4D in front of a Senseair
/// LP8. The LP8 answers a write frame with its acknowledgement and RDY high,
/// and the read frame with a reply built from [`SimLp8`].
pub fn attach_co2_module(bus: &mut MockBus, channel: usize) -> Arc<Mutex<SimLp8>> {
    let lp8 = Arc::new(Mutex::new(SimLp8::default()));
    bus.attach(
        channel,
        crate::co2::EXPANDER,
        Box::new(SimCo2Expander(lp8.clone())),
    );
    bus.attach(channel, crate::co2::UART, Box::new(SimCo2Uart(lp8.clone())));
    lp8
}

/// The simulated LP8 and what the driver wrote to it.
#[derive(Debug)]
pub struct SimLp8 {
    /// The filtered, pressure compensated value it reports (RAM 0xAA).
    pub ppm: i16,
    /// Error status 0 and 1 (RAM 0xA7, 0xA6) of the next replies.
    pub status: [u8; 2],
    /// Calculation control, sensor state and pressure of every write frame.
    pub writes: Vec<(u8, [u8; crate::sensors::lp8::STATE_LEN], u16)>,
    expander: [u8; 4],
    pointer: usize,
    uart: [u8; 16],
    rx: std::collections::VecDeque<u8>,
    tx: Vec<u8>,
    rdy_high: bool,
}

impl Default for SimLp8 {
    fn default() -> Self {
        Self {
            ppm: 650,
            status: [0, 0],
            writes: Vec::new(),
            expander: [0xFF, 0xFF, 0x00, 0xFF],
            pointer: 0,
            uart: [0; 16],
            rx: Default::default(),
            tx: Vec::new(),
            rdy_high: false,
        }
    }
}

impl SimLp8 {
    /// P3 as an output pulls the enable line low: the sensor is on.
    fn enabled(&self) -> bool {
        self.expander[3] & 0x08 == 0
    }

    fn received(&mut self) {
        use crate::sensors::lp8::{STATE_LEN, WRITE_ACK, crc16, read_frame};
        if self.tx.len() >= 33 && self.tx[..2] == [0xFE, 0x41] {
            let frame: Vec<u8> = self.tx.drain(..33).collect();
            if crc16(&frame) == 0 {
                let mut state = [0u8; STATE_LEN];
                state.copy_from_slice(&frame[6..6 + STATE_LEN]);
                let pressure = u16::from_be_bytes([frame[29], frame[30]]);
                self.writes.push((frame[5], state, pressure));
                self.rx.extend(WRITE_ACK);
                self.rdy_high = true;
            }
        } else if self.tx == read_frame() {
            self.tx.clear();
            let Some(&(control, mut state, _)) = self.writes.last() else {
                return;
            };
            // The state changes with every measurement: a counter in byte 0.
            state[0] = state[0].wrapping_add(1);
            let mut ram = [0u8; 44];
            let mut put = |addr: usize, bytes: &[u8]| {
                ram[addr - 0x80..addr - 0x80 + bytes.len()].copy_from_slice(bytes);
            };
            put(0x80, &[control]);
            put(0x81, &state);
            put(0x9A, &(self.ppm + 20).to_be_bytes());
            put(0x9C, &(self.ppm + 10).to_be_bytes());
            put(0x9E, &2350i16.to_be_bytes());
            put(0xA0, &3300u16.to_be_bytes());
            put(0xA2, &3150u16.to_be_bytes());
            put(0xA6, &[self.status[1]]);
            put(0xA7, &[self.status[0]]);
            put(0xA8, &(self.ppm + 5).to_be_bytes());
            put(0xAA, &self.ppm.to_be_bytes());
            let mut reply = vec![0xFE, 0x44, 0x2C];
            reply.extend_from_slice(&ram);
            let crc = crc16(&reply).to_le_bytes();
            reply.extend_from_slice(&crc);
            self.rx.extend(reply);
        }
    }
}

struct SimCo2Expander(Arc<Mutex<SimLp8>>);

impl SimDevice for SimCo2Expander {
    fn write(&mut self, data: &[u8]) -> Result<(), BusError> {
        let mut s = self.0.lock().map_err(|_| BusError::Timeout)?;
        let Some((&cmd, rest)) = data.split_first() else {
            return Ok(());
        };
        if cmd > 3 {
            return Err(BusError::DataNack);
        }
        s.pointer = usize::from(cmd);
        if let Some(&v) = rest.first()
            && cmd != 0
        {
            let was = s.enabled();
            let pointer = s.pointer;
            s.expander[pointer] = v;
            if s.enabled() && !was {
                // Powered up: a new measurement, RDY low until it is done.
                s.rdy_high = false;
                s.tx.clear();
            }
        }
        Ok(())
    }

    fn read(&mut self, buf: &mut [u8]) -> Result<(), BusError> {
        let s = self.0.lock().map_err(|_| BusError::Timeout)?;
        let rdy = !s.enabled() || s.rdy_high;
        let value = match s.pointer {
            0 => 0x7F | if rdy { 0x80 } else { 0 },
            p => s.expander[p],
        };
        buf.fill(value);
        Ok(())
    }
}

struct SimCo2Uart(Arc<Mutex<SimLp8>>);

impl SimDevice for SimCo2Uart {
    fn write(&mut self, data: &[u8]) -> Result<(), BusError> {
        let mut s = self.0.lock().map_err(|_| BusError::Timeout)?;
        let Some((&reg, rest)) = data.split_first() else {
            return Ok(());
        };
        let index = usize::from(reg >> 3) & 0x0F;
        // LCR 0x80 or 0xBF selects the divisor and enhanced registers.
        let special = s.uart[3] == 0x80 || s.uart[3] == 0xBF;
        if index == 0 && !special {
            if s.enabled() {
                s.tx.extend_from_slice(rest);
                s.received();
            }
        } else if let Some(&v) = rest.first() {
            if index == 2 && !special && v & 0x02 != 0 {
                s.rx.clear();
            }
            s.uart[index] = v;
        }
        s.uart[15] = reg;
        Ok(())
    }

    fn read(&mut self, buf: &mut [u8]) -> Result<(), BusError> {
        let mut s = self.0.lock().map_err(|_| BusError::Timeout)?;
        match usize::from(s.uart[15] >> 3) & 0x0F {
            0 => {
                for b in buf.iter_mut() {
                    *b = s.rx.pop_front().unwrap_or(0);
                }
            }
            8 => buf.fill(64),
            9 => buf.fill(u8::try_from(s.rx.len()).unwrap_or(u8::MAX)),
            i => buf.fill(s.uart[i]),
        }
        Ok(())
    }
}

/// The Sensirion SGPC3 of the VOC-LP Tag (0x58).
#[derive(Debug, Default)]
pub struct SimSgpc3 {
    /// TVOC the next measurements report, in ppb.
    pub tvoc: u16,
    pub baseline: u16,
    /// Commands received, with their argument word if any.
    pub commands: Vec<([u8; 2], Option<u16>)>,
    /// Stops acknowledging, as if unplugged.
    pub mute: bool,
    last: [u8; 2],
}

/// Attach a simulated SGPC3 and return its state.
pub fn attach_sgpc3(bus: &mut MockBus, channel: usize) -> Arc<Mutex<SimSgpc3>> {
    let sgp = Arc::new(Mutex::new(SimSgpc3 {
        tvoc: 42,
        baseline: 0x8123,
        ..SimSgpc3::default()
    }));
    bus.attach(
        channel,
        crate::sensors::sgpc3::ADDR,
        Box::new(SimSgpc3Device(sgp.clone())),
    );
    sgp
}

struct SimSgpc3Device(Arc<Mutex<SimSgpc3>>);

impl SimDevice for SimSgpc3Device {
    fn write(&mut self, data: &[u8]) -> Result<(), BusError> {
        let mut s = self.0.lock().map_err(|_| BusError::Timeout)?;
        if s.mute {
            return Err(BusError::AddrNack);
        }
        let [a, b, rest @ ..] = data else {
            return Err(BusError::DataNack);
        };
        let word = match rest {
            [hi, lo, crc] if crc8(&[*hi, *lo], 0xFF) == *crc => {
                Some(u16::from_be_bytes([*hi, *lo]))
            }
            [] => None,
            _ => return Err(BusError::DataNack),
        };
        if [*a, *b] == [0x20, 0x1E]
            && let Some(w) = word
        {
            s.baseline = w;
        }
        s.last = [*a, *b];
        s.commands.push(([*a, *b], word));
        Ok(())
    }

    fn read(&mut self, buf: &mut [u8]) -> Result<(), BusError> {
        let s = self.0.lock().map_err(|_| BusError::Timeout)?;
        if s.mute {
            return Err(BusError::AddrNack);
        }
        let word = match s.last {
            [0x20, 0x2F] => 0x1006,
            [0x20, 0x08] => s.tvoc,
            [0x20, 0x15] => s.baseline,
            _ => return Err(BusError::AddrNack),
        };
        let [hi, lo] = word.to_be_bytes();
        let reply = [hi, lo, crc8(&[hi, lo], 0xFF)];
        for (b, r) in buf.iter_mut().zip(reply.iter().cycle()) {
            *b = *r;
        }
        Ok(())
    }
}

/// The u-blox receiver of the TOWER GPS Module (0x42): it answers a NAV-PVT
/// poll with the position in [`SimGps`], and CFG-PRT with an acknowledgement.
#[derive(Debug, Default)]
pub struct SimGps {
    /// Latitude and longitude in degrees and altitude in metres; `None`: no fix.
    pub position: Option<(f64, f64, f64)>,
    pub satellites: u8,
    /// NAV-PVT polls left unanswered, as a real receiver does for up to two
    /// minutes after power-up.
    pub silent_polls: u32,
    /// UBX frames received, as class and id.
    pub received: Vec<(u8, u8)>,
    stream: std::collections::VecDeque<u8>,
    pointer: u8,
}

/// Attach a GPS Module (its expander and receiver) and return the receiver.
pub fn attach_gps_module(bus: &mut MockBus, channel: usize) -> Arc<Mutex<SimGps>> {
    let gps = Arc::new(Mutex::new(SimGps::default()));
    bus.attach(channel, crate::gps::EXPANDER, Box::new(SimTca9534a::new()));
    bus.attach(
        channel,
        crate::gps::RECEIVER,
        Box::new(SimGpsReceiver(gps.clone())),
    );
    gps
}

impl SimGps {
    fn nav_pvt(&self) -> Vec<u8> {
        let mut p = vec![0u8; 92];
        p[4..6].copy_from_slice(&2026u16.to_le_bytes());
        p[6..11].copy_from_slice(&[10, 17, 20, 0, 0]);
        p[11] = 0x07;
        p[23] = self.satellites;
        if let Some((lat, lon, alt)) = self.position {
            p[20] = 3;
            p[21] = 0x01;
            p[24..28].copy_from_slice(&((lon * 1e7).round() as i32).to_le_bytes());
            p[28..32].copy_from_slice(&((lat * 1e7).round() as i32).to_le_bytes());
            p[36..40].copy_from_slice(&((alt * 1000.0).round() as i32).to_le_bytes());
            p[40..44].copy_from_slice(&2_500u32.to_le_bytes());
        }
        crate::gps::ubx(0x01, 0x07, &p)
    }
}

struct SimGpsReceiver(Arc<Mutex<SimGps>>);

impl SimDevice for SimGpsReceiver {
    fn write(&mut self, data: &[u8]) -> Result<(), BusError> {
        let mut s = self.0.lock().map_err(|_| BusError::Timeout)?;
        match data {
            [reg] => s.pointer = *reg,
            [0xB5, 0x62, class, id, ..] => {
                let (class, id) = (*class, *id);
                s.received.push((class, id));
                let reply = match (class, id) {
                    (0x01, 0x07) if s.silent_polls > 0 => {
                        s.silent_polls -= 1;
                        Vec::new()
                    }
                    (0x01, 0x07) => s.nav_pvt(),
                    (0x06, _) => crate::gps::ubx(0x05, 0x01, &[class, id]),
                    _ => Vec::new(),
                };
                s.stream.extend(reply);
            }
            _ => {}
        }
        Ok(())
    }

    fn read(&mut self, buf: &mut [u8]) -> Result<(), BusError> {
        let mut s = self.0.lock().map_err(|_| BusError::Timeout)?;
        if s.pointer == 0xFD {
            let n = u16::try_from(s.stream.len())
                .unwrap_or(u16::MAX)
                .to_be_bytes();
            for (b, v) in buf.iter_mut().zip(n) {
                *b = v;
            }
        } else {
            for b in buf.iter_mut() {
                *b = s.stream.pop_front().unwrap_or(0xFF);
            }
        }
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

/// The NFC Tag's NTAG I2C plus: 16-byte blocks, block 0 with the NXP
/// manufacturer ID and a capability container, user memory from block 1 (a
/// fresh tag holds an empty NDEF message). The memory is shared, so a test
/// can look at it.
pub struct SimNtag {
    memory: Arc<Mutex<Vec<u8>>>,
    pointer: u8,
}

impl SimNtag {
    const BLOCKS: usize = 0x3B;

    pub fn new() -> Self {
        let mut memory = vec![0u8; Self::BLOCKS * 16];
        memory[..16].copy_from_slice(&[
            0x04, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x80, 0, 0, 0, 0, 0xE1, 0x10, 0x6D, 0x00,
        ]);
        memory[16..19].copy_from_slice(&[0x03, 0x00, 0xFE]);
        Self {
            memory: Arc::new(Mutex::new(memory)),
            pointer: 0,
        }
    }

    /// Without a capability container (not NDEF formatted).
    pub fn unformatted() -> Self {
        let tag = Self::new();
        if let Ok(mut m) = tag.memory.lock() {
            m[12..16].fill(0);
            m[16..19].fill(0);
        }
        tag
    }

    pub fn memory(&self) -> Arc<Mutex<Vec<u8>>> {
        self.memory.clone()
    }
}

impl Default for SimNtag {
    fn default() -> Self {
        Self::new()
    }
}

impl SimDevice for SimNtag {
    fn write(&mut self, data: &[u8]) -> Result<(), BusError> {
        let (&block, rest) = data.split_first().ok_or(BusError::DataNack)?;
        if usize::from(block) >= Self::BLOCKS {
            return Err(BusError::DataNack);
        }
        self.pointer = block;
        match rest.len() {
            0 => Ok(()),
            16 if block > 0 => {
                let start = usize::from(block) * 16;
                if let Ok(mut m) = self.memory.lock() {
                    m[start..start + 16].copy_from_slice(rest);
                }
                Ok(())
            }
            _ => Err(BusError::DataNack),
        }
    }

    fn read(&mut self, buf: &mut [u8]) -> Result<(), BusError> {
        let start = usize::from(self.pointer) * 16;
        if let Ok(m) = self.memory.lock() {
            copy_out(buf, &m[start..start + 16]);
        }
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

/// A simulated bridge whose environment drifts slowly, for running the app
/// without hardware: a Climate Module R2, a Relay Module, a CO2 Module, a
/// VOC-LP Tag and an NFC Tag on channel 0, and the Humidity, Lux Meter and
/// Barometer Tags (somewhere a bit cooler and brighter) and a GPS Module with
/// a fix on channel 1.
pub struct SimulatedBridge {
    bus: MockBus,
    env: SharedEnv,
    tags: SharedEnv,
    lp8: Arc<Mutex<SimLp8>>,
    sgp: Arc<Mutex<SimSgpc3>>,
    gps: Arc<Mutex<SimGps>>,
    seed: u64,
}

impl SimulatedBridge {
    pub fn new() -> Self {
        let env: SharedEnv = Arc::new(Mutex::new(Values::default()));
        let tags: SharedEnv = Arc::new(Mutex::new(Values::default()));
        let mut bus = MockBus::with_climate(ClimateRevision::R2, 0, env.clone());
        bus.attach(0, crate::relay::ADDR_DEFAULT, Box::new(SimTca9534a::new()));
        bus.attach(1, 0x40, Box::new(SimSht20::new(tags.clone())));
        bus.attach(1, 0x44, Box::new(SimOpt3001::new(tags.clone())));
        bus.attach(1, 0x60, Box::new(SimMpl3115a2::new(tags.clone())));
        bus.attach(0, crate::nfc::ADDRESSES[0], Box::new(SimNtag::new()));
        let lp8 = attach_co2_module(&mut bus, 0);
        let sgp = attach_sgpc3(&mut bus, 0);
        let gps = attach_gps_module(&mut bus, 1);
        let mut s = Self {
            bus,
            env,
            tags,
            lp8,
            sgp,
            gps,
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
        let n = [self.noise(), self.noise(), self.noise(), self.noise()];
        let tags = simulated_values(wall - 1800.0, n);
        if let Ok(mut env) = self.env.lock() {
            *env = values;
        }
        if let Ok(mut env) = self.tags.lock() {
            *env = Values {
                temperature: tags.temperature.map(|t| t - 3.5),
                humidity: tags.humidity.map(|h| (h + 9.0).min(100.0)),
                illuminance: tags.illuminance.map(|l| l * 1.8),
                pressure: tags.pressure.map(|p| p + 12.0),
                co2: None,
                voc: None,
            };
        }
        // People in the room during the day: more CO2 and VOC.
        let local = (wall + 7200.0) % 86_400.0 / 86_400.0 * std::f64::consts::TAU;
        let occupied = (local - 2.0).sin().max(0.0);
        let (n1, n2, n3) = (self.noise(), self.noise(), self.noise());
        if let Ok(mut lp8) = self.lp8.lock() {
            lp8.ppm = (520.0 + 420.0 * occupied + 20.0 * n1) as i16;
        }
        if let Ok(mut sgp) = self.sgp.lock() {
            sgp.tvoc = (30.0 + 90.0 * occupied + 10.0 * n2).max(0.0) as u16;
        }
        if let Ok(mut gps) = self.gps.lock() {
            gps.position = Some((50.771168 + n3 * 2e-5, 15.056328 - n3 * 2e-5, 374.5 + n3));
            gps.satellites = 9;
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
        co2: None,
        voc: None,
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
