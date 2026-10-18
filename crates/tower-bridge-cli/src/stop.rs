//! Ctrl+C, SIGTERM and SIGHUP, one handler for the whole process: a command
//! asks for a channel to wait on, or whether a stop was asked for while it
//! waited for something else. Stopping releases the bridge cleanly (LED off,
//! sensors stopped, a relay pulse switched back).

use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};

use crossbeam_channel::{Receiver, Sender};

static REQUESTED: AtomicBool = AtomicBool::new(false);
static CHANNEL: OnceLock<(Sender<()>, Receiver<()>)> = OnceLock::new();

fn channel() -> &'static (Sender<()>, Receiver<()>) {
    CHANNEL.get_or_init(|| {
        let (tx, rx) = crossbeam_channel::bounded(1);
        let signal = tx.clone();
        if let Err(e) = ctrlc::set_handler(move || {
            REQUESTED.store(true, Ordering::Relaxed);
            let _ = signal.try_send(());
        }) {
            tracing::warn!("cannot catch Ctrl+C: {e}");
        }
        (tx, rx)
    })
}

/// Catch the signals from now on.
pub fn install() {
    let _ = channel();
}

/// Fires once when a stop is asked for.
pub fn receiver() -> Receiver<()> {
    channel().1.clone()
}

/// A stop was asked for.
pub fn requested() -> bool {
    install();
    REQUESTED.load(Ordering::Relaxed)
}
