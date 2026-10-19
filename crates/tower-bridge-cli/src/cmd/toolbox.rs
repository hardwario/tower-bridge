//! `toolbox`: low-level tools for bringing up hardware. They open the bridge
//! directly, without the service. `probe` prints readings in the format of
//! hardwario/enmon.

use std::time::Duration;

use anyhow::{Context, Result, bail};
use clap::{Args, Subcommand, ValueEnum};
use tower_bridge_core::bus::discovery::{BackendPref, Discovery};
use tower_bridge_core::bus::mock::SimulatedBridge;
use tower_bridge_core::bus::{BusError, Clock, I2cBus, SystemClock};
use tower_bridge_core::climate::{ChannelMode, Sensors, select_channel};
use tower_bridge_core::model::{DeviceState, STANDARD_SEA_LEVEL_PA, Values, altitude};
use tower_bridge_service::lock::InstanceLock;

use crate::access::Ctx;
use crate::exit;

#[derive(Args)]
pub struct ToolboxArgs {
    /// Transport to use.
    #[arg(long, global = true, value_enum, default_value_t = BackendArg::Auto)]
    backend: BackendArg,
    /// I²C switch channels to use (automatic: both).
    #[arg(long, global = true, value_enum, default_value_t = ChannelArg::Auto)]
    channel: ChannelArg,
    /// Hex-dump every FT260 report to stderr.
    #[arg(long, global = true)]
    trace: bool,
    #[command(subcommand)]
    tool: Tool,
}

#[derive(Subcommand)]
enum Tool {
    /// List connected bridges.
    Scan,
    /// Find the sensors, run measurement cycles and print the values of each
    /// channel in the format of hardwario/enmon.
    Probe {
        /// Number of cycles (timing is printed for each).
        #[arg(long, default_value_t = 1, value_parser = clap::value_parser!(u32).range(1..=1000))]
        cycles: u32,
    },
    /// Switch the bridge LED.
    Led {
        #[arg(value_enum)]
        state: LedState,
    },
    /// List I²C addresses that acknowledge on a switch channel (a read, or a
    /// write of register pointer 0 for chips that ignore a bare read).
    I2cScan,
    /// Identify the chips of the TOWER modules and tags on each switch channel.
    I2cIdentify,
    /// Read bytes from a device: write the register (unless omitted), STOP,
    /// then read. On the channel given with --channel (default 0).
    I2cRead {
        /// 7-bit address, e.g. 0x38
        #[arg(value_parser = parse_byte)]
        address: u8,
        /// Register to read from
        #[arg(value_parser = parse_byte)]
        register: Option<u8>,
        /// Number of bytes
        #[arg(long, default_value_t = 1, value_parser = clap::value_parser!(u16).range(1..=60))]
        len: u16,
    },
    /// Write bytes to a device, e.g. `i2c-write 0x38 0x03 0xff`. On the
    /// channel given with --channel (default 0).
    I2cWrite {
        #[arg(value_parser = parse_byte)]
        address: u8,
        #[arg(value_parser = parse_byte, required = true)]
        bytes: Vec<u8>,
    },
    /// Find the TOWER CO2 Module and measure (the first charge of its
    /// capacitor takes a minute).
    Co2 {
        /// Measurements to take, 16 s apart.
        #[arg(long, default_value_t = 3)]
        count: u32,
        /// Seconds to charge the capacitor first.
        #[arg(long, default_value_t = 60)]
        charge: u64,
        /// Ambient pressure in hPa for the compensation.
        #[arg(long)]
        pressure: Option<f64>,
        /// File to keep the sensor state in between runs.
        #[arg(long)]
        state: Option<std::path::PathBuf>,
    },
    /// Find the TOWER VOC-LP Tag and measure TVOC every 30 s.
    Voc {
        #[arg(long, default_value_t = 3)]
        count: u32,
    },
    /// Find the TOWER GPS Module and print what the receiver reports every 5 s.
    Gps {
        #[arg(long, default_value_t = 6)]
        count: u32,
    },
    /// Look for an NTAG I²C plus at every address (its address can be
    /// changed), on the switch channels and in front of the switch.
    NfcScan,
}

#[derive(Clone, Copy, ValueEnum)]
enum BackendArg {
    Auto,
    Hid,
    I2cDev,
}

#[derive(Clone, Copy, ValueEnum)]
enum ChannelArg {
    Auto,
    #[value(name = "0")]
    Zero,
    #[value(name = "1")]
    One,
}

#[derive(Clone, Copy, ValueEnum)]
enum LedState {
    On,
    Off,
}

impl From<BackendArg> for BackendPref {
    fn from(b: BackendArg) -> Self {
        match b {
            BackendArg::Auto => BackendPref::Auto,
            BackendArg::Hid => BackendPref::Hid,
            BackendArg::I2cDev => BackendPref::I2cDev,
        }
    }
}

impl From<ChannelArg> for ChannelMode {
    fn from(c: ChannelArg) -> Self {
        match c {
            ChannelArg::Auto => ChannelMode::Auto,
            ChannelArg::Zero => ChannelMode::Channel0,
            ChannelArg::One => ChannelMode::Channel1,
        }
    }
}

/// The options every tool needs.
struct Cli {
    backend: BackendArg,
    channel: ChannelArg,
    simulate: bool,
}

pub fn run(ctx: &Ctx, args: ToolboxArgs) -> Result<()> {
    let filter = if args.trace {
        "warn,ft260=trace"
    } else {
        "warn"
    };
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(filter)),
        )
        .with_writer(std::io::stderr)
        .try_init();
    let cli = Cli {
        backend: args.backend,
        channel: args.channel,
        simulate: ctx.simulate,
    };
    // Listing bridges opens none; everything else needs the bridge to itself.
    if !matches!(args.tool, Tool::Scan)
        && !cli.simulate
        && let Some(holder) = InstanceLock::holder(&ctx.paths()?.lock_file())
    {
        return Err(crate::failure(
            exit::BUSY,
            format!("{holder} is running and holds the bridge"),
            Some("stop it to use the toolbox"),
        ));
    }
    match args.tool {
        Tool::Scan if cli.simulate => {
            println!(
                "Simulated  simulated  serial=-  interface=0  usage_page=0xff00  product=TOWER Bridge Module  usable"
            );
            Ok(())
        }
        Tool::Scan => scan(cli.backend.into()),
        Tool::Probe { cycles } => probe(&cli, cycles),
        Tool::Led { state } => {
            let mut bus = open(&cli)?;
            bus.set_led(matches!(state, LedState::On))
                .context("LED write failed")
        }
        Tool::I2cScan => i2c_scan(&cli),
        Tool::I2cIdentify => i2c_identify(&cli),
        Tool::I2cRead {
            address,
            register,
            len,
        } => i2c_read(&cli, address, register, len),
        Tool::I2cWrite { address, ref bytes } => i2c_write(&cli, address, bytes),
        Tool::Co2 {
            count,
            charge,
            pressure,
            ref state,
        } => co2(&cli, count, charge, pressure, state.clone()),
        Tool::Voc { count } => voc(&cli, count),
        Tool::Gps { count } => gps(&cli, count),
        Tool::NfcScan => nfc_scan(&cli),
    }
}

fn co2(
    cli: &Cli,
    count: u32,
    charge: u64,
    pressure: Option<f64>,
    state: Option<std::path::PathBuf>,
) -> Result<()> {
    use tower_bridge_core::co2::Co2Module;
    let mut bus = open(cli)?;
    let clock = SystemClock::new();
    let channels: &[u8] = match cli.channel {
        ChannelArg::Auto => &[0, 1],
        ChannelArg::Zero => &[0],
        ChannelArg::One => &[1],
    };
    let mut found = None;
    for &ch in channels {
        select_channel(bus.as_mut(), ch).context("I²C switch not responding")?;
        if Co2Module::probe(bus.as_mut())? {
            found = Some(ch);
            break;
        }
    }
    let Some(ch) = found else {
        return Err(crate::failure(
            exit::NOT_FOUND,
            "no TOWER CO2 Module found (expander 0x38 and UART 0x4d)",
            None,
        ));
    };
    println!("TOWER CO2 Module on channel {ch}; charging the capacitor for {charge} s");
    let mut module = Co2Module::start(bus.as_mut(), &clock, ch, state)?;
    std::thread::sleep(Duration::from_secs(charge));
    let mut idle = |_: &mut dyn I2cBus, until: Duration| {
        clock.sleep_until(until);
        false
    };
    for i in 0..count {
        if i > 0 {
            std::thread::sleep(tower_bridge_core::co2::MIN_PERIOD);
        }
        let started = std::time::Instant::now();
        match module.measure(bus.as_mut(), &clock, &mut idle, pressure.map(|h| h * 100.0)) {
            Ok(ppm) => {
                let took = started.elapsed().as_millis();
                if let Some(m) = module.last() {
                    println!(
                        "{}: {} ppm (filtered {}, unfiltered {} / compensated {}), {:.2} °C, VCAP {:.2} / {:.2} V, status {:02x?}, {took} ms",
                        i + 1,
                        ppm.map_or("-".into(), |v| format!("{v:.0}")),
                        m.filtered,
                        m.co2,
                        m.co2_pressure,
                        m.temperature,
                        m.vcap1,
                        m.vcap2,
                        m.status
                    );
                }
            }
            Err(e) => println!("{}: failed: {e}", i + 1),
        }
    }
    module.stop(bus.as_mut())?;
    Ok(())
}

/// The first channel (of those asked for) where `probe` finds something;
/// that channel stays selected.
fn find_channel(
    cli: &Cli,
    bus: &mut dyn I2cBus,
    probe: &mut dyn FnMut(&mut dyn I2cBus) -> Result<bool, BusError>,
) -> Result<Option<u8>> {
    let channels: &[u8] = match cli.channel {
        ChannelArg::Auto => &[0, 1],
        ChannelArg::Zero => &[0],
        ChannelArg::One => &[1],
    };
    for &ch in channels {
        select_channel(bus, ch).context("I²C switch not responding")?;
        if probe(bus)? {
            return Ok(Some(ch));
        }
    }
    Ok(None)
}

fn voc(cli: &Cli, count: u32) -> Result<()> {
    use tower_bridge_core::sensors::sgpc3::{self, Sgpc3};
    let mut bus = open(cli)?;
    let clock = SystemClock::new();
    let Some(ch) = find_channel(cli, bus.as_mut(), &mut |b| Ok(sgpc3::probe(b)?.is_some()))? else {
        return Err(crate::failure(
            exit::NOT_FOUND,
            "no TOWER VOC-LP Tag found (SGPC3 at 0x58)",
            None,
        ));
    };
    // No stored baseline here, so the sensor always starts with its preheat.
    println!(
        "TOWER VOC-LP Tag on channel {ch}; a value every 30 s once it has preheated (up to 5 minutes)"
    );
    let mut sensor = Sgpc3::start(bus.as_mut(), &clock, ch, None)?;
    let mut done = 0;
    while done < count {
        clock.sleep_until(sensor.next());
        let ppb = sensor.service(bus.as_mut(), &clock, None)?;
        if sensor.gone() {
            bail!("the SGPC3 does not answer");
        }
        match (ppb, sensor.preheat_left(clock.now())) {
            (Some(ppb), _) => {
                done += 1;
                println!("{done}: {ppb:.0} ppb");
            }
            (None, Some(left)) => println!("preheating, at most {} s more", left.as_secs()),
            (None, None) => {
                done += 1;
                println!("{done}: no value");
            }
        }
    }
    Ok(())
}

fn gps(cli: &Cli, count: u32) -> Result<()> {
    use tower_bridge_core::gps::GpsModule;
    let mut bus = open(cli)?;
    let clock = SystemClock::new();
    let Some(ch) = find_channel(cli, bus.as_mut(), &mut |b| GpsModule::probe(b))? else {
        return Err(crate::failure(
            exit::NOT_FOUND,
            "no TOWER GPS Module found (expander 0x21)",
            None,
        ));
    };
    println!("TOWER GPS Module on channel {ch}");
    let mut module = GpsModule::start(bus.as_mut(), &clock, ch)?;
    let mut polls = 0;
    while polls < count {
        clock.sleep_until(module.next());
        module.service(bus.as_mut(), &clock)?;
        if let Some(info) = module.info() {
            polls += 1;
            println!("{polls}: {}", super::status::gps_summary(info));
        }
        if module.gone() {
            bail!("the GPS receiver does not answer");
        }
    }
    module.stop(bus.as_mut())?;
    Ok(())
}

fn parse_byte(s: &str) -> Result<u8, String> {
    let s = s.trim();
    let r = match s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        Some(hex) => u8::from_str_radix(hex, 16),
        None => s.parse(),
    };
    r.map_err(|e| format!("{s}: {e}"))
}

/// The one channel a raw transfer goes to (`auto` means channel 0).
fn raw_channel(cli: &Cli, bus: &mut dyn I2cBus) -> Result<()> {
    let ch = if matches!(cli.channel, ChannelArg::One) {
        1
    } else {
        0
    };
    select_channel(bus, ch).context("I²C switch not responding")
}

fn i2c_read(cli: &Cli, address: u8, register: Option<u8>, len: u16) -> Result<()> {
    let mut bus = open(cli)?;
    raw_channel(cli, bus.as_mut())?;
    if let Some(reg) = register {
        bus.write(address, &[reg])
            .with_context(|| format!("write to 0x{address:02x}"))?;
    }
    let mut buf = vec![0u8; usize::from(len)];
    bus.read(address, &mut buf)
        .with_context(|| format!("read from 0x{address:02x}"))?;
    println!(
        "{}",
        buf.iter()
            .map(|b| format!("{b:02x}"))
            .collect::<Vec<_>>()
            .join(" ")
    );
    Ok(())
}

fn i2c_write(cli: &Cli, address: u8, bytes: &[u8]) -> Result<()> {
    let mut bus = open(cli)?;
    raw_channel(cli, bus.as_mut())?;
    bus.write(address, bytes)
        .with_context(|| format!("write to 0x{address:02x}"))?;
    println!("ok");
    Ok(())
}

fn nfc_scan(cli: &Cli) -> Result<()> {
    let mut bus = open(cli)?;
    let channels: &[u8] = match cli.channel {
        ChannelArg::Auto => &[0, 1],
        ChannelArg::Zero => &[0],
        ChannelArg::One => &[1],
    };
    let mut passes: Vec<Option<u8>> = channels.iter().copied().map(Some).collect();
    if matches!(cli.channel, ChannelArg::Auto) {
        passes.push(None);
    }
    let mut found = 0;
    for pass in passes {
        match pass {
            Some(ch) => select_channel(bus.as_mut(), ch).context("I²C switch not responding")?,
            None => bus
                .write(tower_bridge_core::climate::SWITCH_ADDR, &[0])
                .context("I²C switch not responding")?,
        }
        let place = pass.map_or("upstream (switch off)".to_string(), |c| {
            format!("channel {c}")
        });
        for addr in 0x08u8..0x78 {
            if addr == tower_bridge_core::climate::SWITCH_ADDR {
                continue;
            }
            if let Some(part) = tower_bridge_core::nfc::identify(bus.as_mut(), addr)? {
                println!("{place}: {part} at 0x{addr:02x}");
                found += 1;
            }
        }
    }
    if found == 0 {
        println!("no NTAG I²C plus answered at any address");
    }
    Ok(())
}

fn open(cli: &Cli) -> Result<Box<dyn I2cBus>> {
    if cli.simulate {
        return Ok(Box::new(SimulatedBridge::new()));
    }
    let mut discovery = Discovery::new(cli.backend.into());
    // Not found, busy or no permission: the same exit codes and hints as
    // the everyday commands.
    let bus = discovery.open_first().map_err(|e| {
        let state = DeviceState::Error {
            kind: e.kind,
            message: e.message.clone(),
        };
        crate::access::device_failure(&state).unwrap_or_else(|| anyhow::anyhow!(e.message))
    })?;
    let info = bus.info();
    eprintln!(
        "Opened {} ({:?}, serial {}, chip {})",
        info.path,
        info.backend,
        info.serial,
        info.chip.as_deref().unwrap_or("?")
    );
    Ok(bus)
}

fn scan(pref: BackendPref) -> Result<()> {
    let mut discovery = Discovery::new(pref);
    let candidates = discovery.scan()?;
    if candidates.is_empty() {
        return Err(crate::failure(
            exit::NOT_FOUND,
            "no TOWER Bridge Module found",
            Some("check the USB cable; on Linux, the udev rule (see docs/linux.md)"),
        ));
    }
    for c in candidates {
        println!(
            "{:<8} {}  serial={}  interface={}  usage_page=0x{:04x}  product={}  {}",
            format!("{:?}", c.backend),
            c.path,
            c.serial.as_deref().unwrap_or("-"),
            c.interface,
            c.usage_page,
            c.product.as_deref().unwrap_or("-"),
            if c.usable { "usable" } else { "not usable" }
        );
    }
    Ok(())
}

fn fmt(v: Option<f64>, decimals: usize) -> String {
    v.map(|x| format!("{x:.decimals$}"))
        .unwrap_or_else(|| "NULL".into())
}

/// enmon-compatible output.
fn print_values(v: &Values) {
    let alt = v.pressure.map(|p| altitude(p, STANDARD_SEA_LEVEL_PA));
    println!("@SENSOR: \"Temperature\",{}", fmt(v.temperature, 2));
    println!("@SENSOR: \"Humidity\",{}", fmt(v.humidity, 1));
    println!("@SENSOR: \"Illuminance\",{}", fmt(v.illuminance, 0));
    println!("@SENSOR: \"Pressure\",{}", fmt(v.pressure, 0));
    println!("@SENSOR: \"Altitude\",{}", fmt(alt, 1));
}

fn probe(cli: &Cli, cycles: u32) -> Result<()> {
    let mut bus = open(cli)?;
    let clock = SystemClock::new();
    let mut sensors = Sensors::new(cli.channel.into());
    sensors.detect(bus.as_mut(), &clock)?;
    if sensors.is_empty() {
        return Err(crate::failure(
            exit::NOT_FOUND,
            "no sensors found on the bridge",
            None,
        ));
    }
    for s in sensors.sources() {
        eprintln!(
            "channel {}: {} {} 0x{:02x} ({})",
            s.channel,
            s.module.name(),
            s.sensor,
            s.address,
            s.metric.as_str()
        );
    }
    let mut failures = 0u32;
    let mut durations = Vec::new();
    for n in 1..=cycles {
        let reports = sensors.measure(bus.as_mut(), &clock)?;
        let mut took = 0;
        let mut measured = false;
        for (channel, report) in &reports {
            for (sensor, err) in &report.errors {
                eprintln!("cycle {n}, channel {channel}: {}: {err}", sensor.name());
            }
            failures += report.errors.len() as u32;
            took += report.duration.as_millis();
            measured |= !report.values.is_empty();
            if reports.len() > 1 {
                println!("# channel {channel}");
            }
            print_values(&report.values);
        }
        durations.push(took);
        eprintln!("Cycle {n} took {took} ms");
        if !measured {
            bail!("all sensors failed");
        }
    }
    if cycles > 1 {
        let steady = &durations[1..];
        let avg = steady.iter().sum::<u128>() / steady.len() as u128;
        let max = steady.iter().max().copied().unwrap_or(0);
        eprintln!(
            "{cycles} cycles, {failures} sensor errors; after the first: avg {avg} ms, max {max} ms"
        );
    }
    Ok(())
}

fn i2c_identify(cli: &Cli) -> Result<()> {
    let mut bus = open(cli)?;
    let channels: &[u8] = match cli.channel {
        ChannelArg::Auto => &[0, 1],
        ChannelArg::Zero => &[0],
        ChannelArg::One => &[1],
    };
    for &ch in channels {
        select_channel(bus.as_mut(), ch).context("I²C switch not responding")?;
        let found = tower_bridge_core::sensors::identify::scan(bus.as_mut())?;
        let list: Vec<String> = found
            .iter()
            .map(|f| format!("0x{:02x} {}", f.address, f.part))
            .collect();
        println!(
            "channel {ch}: {}",
            if list.is_empty() {
                "-".into()
            } else {
                list.join(", ")
            }
        );
    }
    Ok(())
}

fn i2c_scan(cli: &Cli) -> Result<()> {
    let mut bus = open(cli)?;
    let channels: &[u8] = match cli.channel {
        ChannelArg::Auto => &[0, 1],
        ChannelArg::Zero => &[0],
        ChannelArg::One => &[1],
    };
    // Channels behind the switch, then the bus in front of it (switch off).
    let mut passes: Vec<Option<u8>> = channels.iter().copied().map(Some).collect();
    if matches!(cli.channel, ChannelArg::Auto) {
        passes.push(None);
    }
    for pass in passes {
        match pass {
            Some(ch) => select_channel(bus.as_mut(), ch).context("I²C switch not responding")?,
            None => bus
                .write(tower_bridge_core::climate::SWITCH_ADDR, &[0])
                .context("I²C switch not responding")?,
        }
        let mut found = Vec::new();
        // Anything other than "no device here" hints at a wiring or bus problem.
        let mut other: Vec<(u8, BusError)> = Vec::new();
        for addr in 0x08u8..0x78 {
            if addr == tower_bridge_core::climate::SWITCH_ADDR {
                continue;
            }
            let mut b = [0u8; 1];
            // Some chips (TCA9534 expanders, the NTAG) ignore a bare read;
            // setting register pointer 0 is harmless for every chip here.
            match bus.read(addr, &mut b).or_else(|e| match e {
                BusError::AddrNack => bus.write(addr, &[0x00]),
                e => Err(e),
            }) {
                Ok(()) => found.push(addr),
                Err(e) if e.is_fatal() => return Err(e.into()),
                Err(BusError::AddrNack) => {}
                Err(e) => other.push((addr, e)),
            }
        }
        let list: Vec<String> = found.iter().map(|a| format!("0x{a:02x}")).collect();
        let name = pass.map_or("upstream (switch off)".to_string(), |ch| {
            format!("channel {ch}")
        });
        println!(
            "{name}: {}",
            if list.is_empty() {
                "-".into()
            } else {
                list.join(" ")
            }
        );
        if let Some((addr, e)) = other.first() {
            println!(
                "  {} address(es) failed with something other than NACK, first 0x{addr:02x}: {e}",
                other.len()
            );
        }
    }
    Ok(())
}
