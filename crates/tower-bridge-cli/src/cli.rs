//! The command line: commands, options and their help texts.

use std::path::PathBuf;

use clap::{ArgAction, Args, Parser, Subcommand, ValueEnum};

use crate::cmd::toolbox::ToolboxArgs;

const AFTER_HELP: &str = "\
With no command in a terminal, opens the dashboard.

Settings and history are shared with the TOWER Bridge desktop app. While the app
or `tower-bridge-cli run` holds the bridge, commands reach it through its API
server (in the app: Settings, API & MCP; here: `config set api.enabled true`).

Exit codes: 0 ok, 1 error, 2 usage, 3 bridge busy, 4 not found, 130 stopped (Ctrl+C).";

#[derive(Parser)]
#[command(
    name = "tower-bridge-cli",
    version,
    about = "Command line, terminal dashboard and headless service for the HARDWARIO TOWER Bridge Module",
    after_help = AFTER_HELP
)]
pub struct Cli {
    /// Data folder with the settings and history, shared with the desktop app.
    #[arg(long, global = true, value_name = "DIR", env = "TOWER_BRIDGE_DATA_DIR")]
    pub data_dir: Option<PathBuf>,
    /// Use a simulated bridge with every module; its data is kept apart.
    // Any value but 0, false, no or off turns it on, as in the app (`=1`).
    #[arg(long, global = true, env = "TOWER_BRIDGE_SIMULATE", value_parser = clap::builder::FalseyValueParser::new())]
    pub simulate: bool,
    /// Print JSON: one object, or one per line for streams.
    #[arg(long, global = true)]
    pub json: bool,
    /// When to use colours (`auto`: in a terminal, unless NO_COLOR is set).
    #[arg(long, global = true, value_enum, value_name = "WHEN", default_value_t = ColorMode::Auto)]
    pub color: ColorMode,
    /// More log output on stderr; repeat for more.
    #[arg(short, long, global = true, action = ArgAction::Count)]
    pub verbose: u8,
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum ColorMode {
    Auto,
    Always,
    Never,
}

#[derive(Subcommand)]
pub enum Command {
    /// Terminal dashboard: live values, charts, relay, GPS, NFC and alerts (the default in a terminal).
    Dashboard,
    /// Run the service without a window: record history, check alerts, publish to MQTT, serve REST and MCP.
    Run,
    /// Show the bridge, its modules and the latest values.
    Status,
    /// Print readings as they come; Ctrl+C stops.
    Watch {
        /// Stop after this many readings.
        #[arg(long, value_name = "N", value_parser = clap::value_parser!(u32).range(1..))]
        count: Option<u32>,
    },
    /// Show the Relay Module, or switch or pulse it.
    Relay {
        #[command(subcommand)]
        action: Option<RelayAction>,
    },
    /// Show the GPS position and fix.
    Gps,
    /// Show the URL on the NFC Tag, or write a new one.
    Nfc {
        #[command(subcommand)]
        action: Option<NfcAction>,
    },
    /// Blink the LED on the TOWER Bridge Module.
    Identify,
    /// Look for modules again.
    Rescan,
    /// Export stored readings to CSV or XLSX.
    Export(ExportArgs),
    /// Show the alert rules, the active alerts and the recent alert log.
    Alerts {
        /// Entries of the alert log to show.
        #[arg(long, value_name = "N", default_value_t = 20)]
        log: usize,
    },
    /// Show or change the settings, shared with the desktop app.
    Config {
        #[command(subcommand)]
        action: Option<ConfigAction>,
    },
    /// MCP server over stdin and stdout for AI assistants, forwarding to the running service.
    Mcp,
    /// Low-level tools for bringing up hardware; they open the bridge directly.
    Toolbox(ToolboxArgs),
}

#[derive(Subcommand)]
pub enum RelayAction {
    /// Switch the relay to TRUE.
    #[command(name = "true")]
    True,
    /// Switch the relay to FALSE.
    #[command(name = "false")]
    False,
    /// Switch the relay to a state for a while, then back.
    Pulse {
        /// The state to pulse to (default: the opposite of the current one).
        #[arg(value_enum)]
        state: Option<RelayState>,
        /// Length in ms, 500 to 3600000 (default: the setting, 2000 at first).
        #[arg(long, value_name = "MS", value_parser = clap::value_parser!(u64).range(500..=3_600_000))]
        ms: Option<u64>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum RelayState {
    True,
    False,
}

#[derive(Subcommand)]
pub enum NfcAction {
    /// Write a URL that phones open when they touch the tag; https:// is added when missing.
    Write {
        /// At most 240 bytes, e.g. hardwario.com/tower
        url: String,
    },
}

#[derive(Args)]
pub struct ExportArgs {
    /// Output file; .xlsx writes a workbook, anything else CSV. Without it, CSV goes to stdout.
    #[arg(short, long, value_name = "FILE")]
    pub output: Option<PathBuf>,
    /// The last span up to now, e.g. 30m, 24h, 7d (default 24h).
    #[arg(long, value_name = "SPAN", conflicts_with_all = ["from", "to"])]
    pub last: Option<String>,
    /// Start: a date (2026-10-17), a local time (2026-10-17T08:00), RFC 3339 or Unix ms.
    #[arg(long, value_name = "TIME")]
    pub from: Option<String>,
    /// End, in the same forms (default: now).
    #[arg(long, value_name = "TIME")]
    pub to: Option<String>,
}

#[derive(Subcommand)]
pub enum ConfigAction {
    /// Print all settings (the password and token as "(set)").
    Show,
    /// Print one setting or group, e.g. mqtt.host or alerts.temperature; secrets only by their own key.
    Get {
        /// Dotted key, as in settings.json.
        key: String,
    },
    /// Change one setting, e.g. `config set mqtt.enabled true`; only while no service runs.
    Set {
        /// Dotted key, as in settings.json.
        key: String,
        /// New value: JSON (true, 20000, -10, null, "text") or plain text.
        #[arg(allow_hyphen_values = true)]
        value: String,
    },
    /// Print where the settings file is.
    Path,
}
