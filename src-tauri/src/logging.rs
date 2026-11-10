//! Logging to stderr and a daily rolling file in the data directory.

use std::path::Path;

use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

/// Panics go to the log too: the Windows build has no console, and nobody
/// reads the stderr of an app started from the Finder or at login. The
/// threads that restart after a panic (the hub, the acquisition) would
/// otherwise leave only "panicked; restarting" behind.
fn log_panics() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let thread = std::thread::current();
        let location = info
            .location()
            .map(|l| format!(" at {}:{}", l.file(), l.line()))
            .unwrap_or_default();
        tracing::error!(
            "panic in thread {}{location}: {}",
            thread.name().unwrap_or("unnamed"),
            info.payload_as_str().unwrap_or("(no message)")
        );
        previous(info);
    }));
}

pub fn init(logs_dir: &Path) -> Option<WorkerGuard> {
    log_panics();
    let filter = EnvFilter::try_from_env("TOWER_BRIDGE_LOG")
        .unwrap_or_else(|_| EnvFilter::new("info,rumqttc=warn,tao=warn,wry=warn"));
    let _ = std::fs::create_dir_all(logs_dir);
    let file = tracing_appender::rolling::Builder::new()
        .rotation(tracing_appender::rolling::Rotation::DAILY)
        .filename_prefix("tower-bridge")
        .filename_suffix("log")
        .max_log_files(7)
        .build(logs_dir)
        .ok();
    let stderr = tracing_subscriber::fmt::layer().with_writer(std::io::stderr);
    match file {
        Some(appender) => {
            let (writer, guard) = tracing_appender::non_blocking(appender);
            let file_layer = tracing_subscriber::fmt::layer()
                .with_ansi(false)
                .with_writer(writer);
            let _ = tracing_subscriber::registry()
                .with(filter)
                .with(stderr)
                .with(file_layer)
                .try_init();
            Some(guard)
        }
        None => {
            let _ = tracing_subscriber::registry()
                .with(filter)
                .with(stderr)
                .try_init();
            None
        }
    }
}
