//! The commands.

mod config;
mod data;
mod devices;
mod run;
mod status;
pub mod toolbox;
mod watch;

use std::io::IsTerminal;

use anyhow::Result;
use clap::CommandFactory;

use crate::access::Ctx;
use crate::cli::{Cli, Command};
use crate::output::Style;
use crate::{exit, init_logging};

/// How to print: JSON or text, and with colours or not.
#[derive(Clone, Copy, Debug)]
pub struct Out {
    pub json: bool,
    pub style: Style,
}

pub fn run(cli: Cli) -> Result<()> {
    let ctx = Ctx {
        data_dir: cli.data_dir.clone(),
        simulate: cli.simulate,
    };
    let out = Out {
        json: cli.json,
        style: Style::new(cli.color),
    };
    let interactive = std::io::stdin().is_terminal() && std::io::stdout().is_terminal();
    let command = match cli.command {
        Some(c) => c,
        None if interactive => Command::Dashboard,
        None => {
            let _ = Cli::command().print_help();
            return Err(crate::failure(
                exit::USAGE,
                "no command given, and the dashboard needs a terminal",
                Some("use `status` or `watch` in scripts"),
            ));
        }
    };
    // The dashboard, the service and the toolbox set up their own logging.
    if !matches!(
        command,
        Command::Dashboard | Command::Run | Command::Toolbox(_)
    ) {
        init_logging(cli.verbose, false);
    }
    match command {
        Command::Dashboard => crate::tui::run(&ctx, cli.verbose),
        Command::Run => run::service(&ctx, cli.verbose),
        Command::Status => status::status(&ctx, out),
        Command::Watch { count } => watch::watch(&ctx, out, count),
        Command::Relay { action } => devices::relay(&ctx, out, action),
        Command::Gps => devices::gps(&ctx, out),
        Command::Nfc { action } => devices::nfc(&ctx, out, action),
        Command::Identify => status::identify(&ctx, out),
        Command::Rescan => status::rescan(&ctx, out),
        Command::Export(args) => data::export(&ctx, &args),
        Command::Alerts { log } => data::alerts(&ctx, out, log),
        Command::Config { action } => config::config(&ctx, out, action),
        Command::Mcp => run::mcp(&ctx),
        Command::Toolbox(args) => toolbox::run(&ctx, args),
    }
}

/// Print a labelled line of the text output: the label in a fixed column.
fn line(out: Out, label: &str, value: &str) {
    println!(
        "{}{} {value}",
        out.style.bold(label),
        " ".repeat(20usize.saturating_sub(label.chars().count()))
    );
}
