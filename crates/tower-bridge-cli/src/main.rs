//! Command-line diagnostics for the TOWER Bridge Module, used to validate
//! hardware support before the desktop app. `probe` prints readings in the
//! same format as hardwario/enmon.

use std::time::Duration;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand, ValueEnum};
use tower_bridge_core::acquisition::{AcqCommand, AcqConfig, AcqEvent, Acquisition, Opener};
use tower_bridge_core::bus::discovery::{BackendPref, Discovery};
use tower_bridge_core::bus::mock::SimulatedBridge;
use tower_bridge_core::bus::{BusError, I2cBus, SystemClock};
use tower_bridge_core::climate::{ChannelMode, Sensors, select_channel};
use tower_bridge_core::model::{STANDARD_SEA_LEVEL_PA, Values, altitude};
use tower_bridge_core::relay::RelayModule;

#[derive(Parser)]
#[command(version, about = "HARDWARIO TOWER Bridge Module diagnostics")]
struct Cli {
    /// Transport to use.
    #[arg(long, value_enum, default_value_t = BackendArg::Auto, global = true)]
    backend: BackendArg,
    /// I2C switch channels to use (automatic: both).
    #[arg(long, value_enum, default_value_t = ChannelArg::Auto, global = true)]
    channel: ChannelArg,
    /// Hex-dump every FT260 report to stderr.
    #[arg(long, global = true)]
    trace: bool,
    /// Use a simulated bridge instead of hardware.
    #[arg(long, global = true)]
    simulate: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// List connected bridges.
    Scan,
    /// Find the sensors, run measurement cycles and print the values of each
    /// channel.
    Probe {
        /// Number of cycles (timing is printed for each).
        #[arg(long, default_value_t = 1, value_parser = clap::value_parser!(u32).range(1..=1000))]
        cycles: u32,
    },
    /// Measure continuously (with reconnect) and print each reading.
    Watch {
        /// Seconds between measurements.
        #[arg(long, default_value_t = 2, value_parser = clap::value_parser!(u64).range(2..=3600))]
        interval: u64,
    },
    /// Switch the bridge LED.
    Led {
        #[arg(value_enum)]
        state: LedState,
    },
    /// List I2C addresses that acknowledge on a switch channel.
    I2cScan,
    /// Identify the sensor chips on each switch channel.
    I2cIdentify,
    /// Find the Relay Module and show its state or switch it to true or false.
    Relay {
        #[arg(value_enum, default_value_t = RelayAction::Status)]
        action: RelayAction,
    },
}

#[derive(Clone, Copy, ValueEnum)]
enum RelayAction {
    Status,
    True,
    False,
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

fn main() {
    let cli = Cli::parse();
    let filter = if cli.trace {
        "warn,ft260=trace"
    } else {
        "warn"
    };
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(filter)),
        )
        .with_writer(std::io::stderr)
        .init();
    if let Err(e) = run(cli) {
        eprintln!("error: {e:#}");
        std::process::exit(1);
    }
}

fn run(cli: Cli) -> Result<()> {
    match cli.command {
        Command::Scan => scan(cli.backend.into()),
        Command::Probe { cycles } => probe(&cli, cycles),
        Command::Watch { interval } => watch(&cli, Duration::from_secs(interval)),
        Command::Led { state } => {
            let mut bus = open(&cli)?;
            bus.set_led(matches!(state, LedState::On))
                .context("LED write failed")
        }
        Command::I2cScan => i2c_scan(&cli),
        Command::I2cIdentify => i2c_identify(&cli),
        Command::Relay { action } => relay(&cli, action),
    }
}

fn relay(cli: &Cli, action: RelayAction) -> Result<()> {
    let mut bus = open(cli)?;
    let channels: &[u8] = match cli.channel {
        ChannelArg::Auto => &[0, 1],
        ChannelArg::Zero => &[0],
        ChannelArg::One => &[1],
    };
    let Some(mut relay) = RelayModule::detect(bus.as_mut(), channels)? else {
        bail!("no Relay Module found (tried 0x3b, 0x3f, 0x23, 0x27)");
    };
    let info = relay.info();
    println!(
        "Relay Module on channel {} at 0x{:02x}, state {}",
        info.channel,
        info.address,
        info.state.map_or("unknown".to_string(), |s| s.to_string())
    );
    let state = match action {
        RelayAction::Status => return Ok(()),
        RelayAction::True => true,
        RelayAction::False => false,
    };
    relay
        .set(bus.as_mut(), &SystemClock::new(), state)
        .context("switching failed")?;
    println!("switched {state}");
    Ok(())
}

fn open(cli: &Cli) -> Result<Box<dyn I2cBus>> {
    if cli.simulate {
        return Ok(Box::new(SimulatedBridge::new()));
    }
    let mut discovery = Discovery::new(cli.backend.into());
    let bus = discovery.open_first()?;
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

fn opener(cli: &Cli) -> Opener {
    if cli.simulate {
        tower_bridge_core::acquisition::simulated_opener()
    } else {
        tower_bridge_core::acquisition::hardware_opener(cli.backend.into())
    }
}

fn scan(pref: BackendPref) -> Result<()> {
    let mut discovery = Discovery::new(pref);
    let candidates = discovery.scan()?;
    if candidates.is_empty() {
        println!("No TOWER Bridge Module found.");
        return Ok(());
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
        bail!("No sensors found on the bridge");
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

fn watch(cli: &Cli, interval: Duration) -> Result<()> {
    let (tx, rx) = crossbeam_channel::unbounded();
    let acq = Acquisition::spawn(
        AcqConfig {
            interval,
            channel: cli.channel.into(),
        },
        opener(cli),
        Box::new(SystemClock::new()),
        tx,
    )?;
    let commands = acq.sender();
    for ev in rx {
        match ev {
            AcqEvent::Device(state) => eprintln!("device: {state:?}"),
            AcqEvent::Relay(relay) => eprintln!("relay: {relay:?}"),
            AcqEvent::Reading(r) => {
                println!("# ts={} device={} channel={}", r.ts, r.device, r.channel);
                print_values(&r.values);
            }
        }
    }
    let _ = commands.send(AcqCommand::Shutdown);
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
        select_channel(bus.as_mut(), ch).context("I2C switch not responding")?;
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
            Some(ch) => select_channel(bus.as_mut(), ch).context("I2C switch not responding")?,
            None => bus
                .write(tower_bridge_core::climate::SWITCH_ADDR, &[0])
                .context("I2C switch not responding")?,
        }
        let mut found = Vec::new();
        // Anything other than "no device here" hints at a wiring or bus problem.
        let mut other: Vec<(u8, BusError)> = Vec::new();
        for addr in 0x08u8..0x78 {
            if addr == tower_bridge_core::climate::SWITCH_ADDR {
                continue;
            }
            let mut b = [0u8; 1];
            match bus.read(addr, &mut b) {
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
