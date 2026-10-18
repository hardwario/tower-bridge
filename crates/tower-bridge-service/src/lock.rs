//! One running instance per data folder: the desktop app or the CLI's
//! service, never both. Both would want the bridge (which only one program
//! can open) and would overwrite each other's settings.
//!
//! The lock is an OS file lock on `tower-bridge.lock`, so it goes away with
//! the process, also after a crash. The file names the holder for messages.

use std::fs::{File, OpenOptions, TryLockError};
use std::io::{Read, Seek, Write};
use std::path::Path;

/// Held while the instance runs.
#[derive(Debug)]
pub struct InstanceLock {
    _file: File,
}

/// Who holds the lock, as far as the file says (`None` where the platform
/// does not let others read a locked file, as on Windows).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Holder {
    pub program: Option<String>,
    pub pid: Option<u32>,
}

impl std::fmt::Display for Holder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match (&self.program, self.pid) {
            (Some(program), Some(pid)) => write!(f, "{program} (process {pid})"),
            (Some(program), None) => write!(f, "{program}"),
            _ => write!(f, "TOWER Bridge"),
        }
    }
}

impl InstanceLock {
    /// Take the lock for `program`, or say who has it.
    pub fn acquire(path: &Path, program: &str) -> std::io::Result<Result<Self, Holder>> {
        // Not truncated before the lock is ours: it names the holder.
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)?;
        match file.try_lock() {
            Ok(()) => {
                file.set_len(0)?;
                file.rewind()?;
                writeln!(file, "{program} {}", std::process::id())?;
                file.flush()?;
                Ok(Ok(Self { _file: file }))
            }
            Err(TryLockError::WouldBlock) => Ok(Err(read_holder(&mut file))),
            Err(TryLockError::Error(e)) => Err(e),
        }
    }

    /// Who runs now, without taking the lock; `None` when nobody does.
    pub fn holder(path: &Path) -> Option<Holder> {
        let mut file = OpenOptions::new().read(true).write(true).open(path).ok()?;
        match file.try_lock() {
            // Free: released again when `file` closes.
            Ok(()) => None,
            Err(TryLockError::WouldBlock) => Some(read_holder(&mut file)),
            Err(TryLockError::Error(_)) => None,
        }
    }
}

fn read_holder(file: &mut File) -> Holder {
    let mut text = String::new();
    let _ = file.read_to_string(&mut text);
    let mut words = text.split_whitespace();
    Holder {
        program: words.next().map(str::to_string),
        pid: words.next().and_then(|p| p.parse().ok()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_second_instance_learns_who_runs() -> std::io::Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("tower-bridge.lock");
        assert_eq!(InstanceLock::holder(&path), None, "no file yet");
        let first = InstanceLock::acquire(&path, "tower-bridge-cli")?;
        assert!(first.is_ok());
        let second = InstanceLock::acquire(&path, "tower-bridge")?;
        let holder = second.err();
        assert!(holder.is_some(), "locked");
        if cfg!(unix) {
            let pid = std::process::id();
            assert_eq!(
                holder,
                Some(Holder {
                    program: Some("tower-bridge-cli".into()),
                    pid: Some(pid)
                })
            );
            assert_eq!(InstanceLock::holder(&path).and_then(|h| h.pid), Some(pid));
        }
        drop(first);
        assert_eq!(InstanceLock::holder(&path), None, "released");
        assert!(InstanceLock::acquire(&path, "tower-bridge")?.is_ok());
        Ok(())
    }
}
