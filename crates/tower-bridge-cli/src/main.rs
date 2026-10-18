//! `tower-bridge-cli`: the command line, terminal dashboard and headless
//! service of TOWER Bridge, the sibling of the desktop app. It shares the
//! app's data folder (settings and history) and its service
//! (`tower-bridge-service`).
//!
//! Data goes to stdout, messages to stderr. While another instance (the app
//! or `run`) holds the bridge, commands go through its API server.

mod access;
mod cli;
mod cmd;
mod output;
mod remote;
mod tui;

use std::io::{IsTerminal, Write};
use std::process::ExitCode;

use clap::Parser;

/// Exit codes; docs/cli.md lists them too.
pub mod exit {
    pub const ERROR: u8 = 1;
    /// Usage errors, from clap.
    #[allow(dead_code)]
    pub const USAGE: u8 = 2;
    /// Another program holds the bridge (or TOWER Bridge runs without its
    /// API server), or a setting cannot change while the service runs.
    pub const BUSY: u8 = 3;
    /// No bridge, or not the module the command needs.
    pub const NOT_FOUND: u8 = 4;
}

/// An error with its exit code and a way out.
#[derive(Debug)]
pub struct Failure {
    pub code: u8,
    pub message: String,
    pub hint: Option<String>,
}

impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for Failure {}

pub fn failure(code: u8, message: impl Into<String>, hint: Option<&str>) -> anyhow::Error {
    Failure {
        code,
        message: message.into(),
        hint: hint.map(str::to_string),
    }
    .into()
}

fn main() -> ExitCode {
    restore_sigpipe();
    let cli = cli::Cli::parse();
    match cmd::run(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            let (code, hint) = match e.downcast_ref::<Failure>() {
                Some(f) => (f.code, f.hint.clone()),
                None => (exit::ERROR, None),
            };
            // Not `eprintln!`: it panics when stderr is a terminal that is gone.
            let mut stderr = std::io::stderr();
            let _ = writeln!(stderr, "error: {e:#}");
            if let Some(hint) = hint {
                let _ = writeln!(stderr, "hint: {hint}");
            }
            ExitCode::from(code)
        }
    }
}

/// Rust ignores SIGPIPE, which turns a closed pipe (`tower-bridge-cli watch |
/// head`) into a panic in `println!`. The default disposition ends the
/// process quietly, as with any other Unix tool.
#[cfg(unix)]
#[allow(unsafe_code)]
fn restore_sigpipe() {
    // SAFETY: called first thing in main, before any other thread exists;
    // SIG_DFL is a valid disposition for SIGPIPE.
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }
}

#[cfg(not(unix))]
fn restore_sigpipe() {}

/// Log lines on stderr: warnings by default, `-v` for info, `-vv` for debug;
/// `RUST_LOG` overrides. The service logs info by default.
pub fn init_logging(verbose: u8, service: bool) {
    let level = match (verbose, service) {
        (0, false) => "warn",
        (0, true) | (1, _) => "info",
        (2, _) => "debug",
        _ => "trace",
    };
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(level));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .with_ansi(std::io::stderr().is_terminal() && std::env::var_os("NO_COLOR").is_none())
        .with_target(false)
        .try_init();
}
