//! Linux fallback: the kernel `hid-ft260` driver exposes the bridge as an I2C
//! adapter (`/dev/i2c-N`, needs the `i2c-dev` module). Used when the driver
//! has claimed the device and no hidraw node is available. No LED control.

use std::path::{Path, PathBuf};

use i2cdev::core::{I2CMessage, I2CTransfer};
use i2cdev::linux::{LinuxI2CBus, LinuxI2CError, LinuxI2CMessage};

use super::{BusError, I2cBus};
use crate::model::{Backend, DeviceInfo};

const SYSFS_I2C: &str = "/sys/bus/i2c/devices";

/// An FT260 I2C adapter registered by the kernel.
#[derive(Clone, Debug)]
pub struct Adapter {
    /// e.g. `/dev/i2c-7`
    pub dev_path: PathBuf,
    /// Adapter name from sysfs, e.g. `FT260 usb-i2c bridge`.
    pub name: String,
    pub serial: Option<String>,
}

impl Adapter {
    pub fn dev_node_exists(&self) -> bool {
        self.dev_path.exists()
    }
}

/// Find FT260 adapters registered by the kernel.
pub fn list_adapters() -> Vec<Adapter> {
    let Ok(entries) = std::fs::read_dir(SYSFS_I2C) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let file_name = entry.file_name();
        let Some(dir_name) = file_name.to_str() else {
            continue;
        };
        if !dir_name.starts_with("i2c-") {
            continue;
        }
        let Ok(name) = std::fs::read_to_string(entry.path().join("name")) else {
            continue;
        };
        let name = name.trim().to_string();
        if !name.starts_with("FT260") {
            continue;
        }
        out.push(Adapter {
            dev_path: Path::new("/dev").join(dir_name),
            name,
            serial: usb_serial(&entry.path()),
        });
    }
    out.sort_by(|a, b| a.dev_path.cmp(&b.dev_path));
    out
}

/// Walk up the sysfs hierarchy to the USB device and read its serial number.
fn usb_serial(adapter: &Path) -> Option<String> {
    let real = std::fs::canonicalize(adapter).ok()?;
    real.ancestors()
        .find(|p| p.join("idVendor").exists() && p.join("serial").exists())
        .and_then(|p| std::fs::read_to_string(p.join("serial")).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// I2C master on top of `/dev/i2c-N`.
pub struct LinuxI2cDev {
    bus: LinuxI2CBus,
    info: DeviceInfo,
}

impl LinuxI2cDev {
    pub fn open(adapter: &Adapter) -> Result<Self, std::io::Error> {
        let bus = LinuxI2CBus::new(&adapter.dev_path).map_err(std::io::Error::from)?;
        Ok(Self {
            bus,
            info: DeviceInfo {
                backend: Backend::I2cDev,
                serial: adapter.serial.clone().unwrap_or_else(|| "ft260".into()),
                path: adapter.dev_path.display().to_string(),
                chip: Some(adapter.name.clone()),
            },
        })
    }

    fn transfer(&mut self, msgs: &mut [LinuxI2CMessage<'_>]) -> Result<(), BusError> {
        self.bus.transfer(msgs).map(|_| ()).map_err(map_err)
    }
}

fn map_err(e: LinuxI2CError) -> BusError {
    let errno = match &e {
        LinuxI2CError::Errno(n) => Some(*n),
        LinuxI2CError::Io(io) => io.raw_os_error(),
    };
    match errno {
        // EIO, ENXIO, EREMOTEIO: the kernel driver reports NACKs this way.
        Some(5) | Some(6) | Some(121) => BusError::AddrNack,
        // ETIMEDOUT
        Some(110) => BusError::Timeout,
        // EAGAIN, EBUSY
        Some(11) | Some(16) => BusError::Bus(0),
        _ => BusError::Io(e.to_string()),
    }
}

impl I2cBus for LinuxI2cDev {
    fn write(&mut self, addr: u8, data: &[u8]) -> Result<(), BusError> {
        let mut msgs = [LinuxI2CMessage::write(data).with_address(u16::from(addr))];
        self.transfer(&mut msgs)
    }

    fn read(&mut self, addr: u8, buf: &mut [u8]) -> Result<(), BusError> {
        let mut msgs = [LinuxI2CMessage::read(buf).with_address(u16::from(addr))];
        self.transfer(&mut msgs)
    }

    fn write_read(&mut self, addr: u8, wr: &[u8], rd: &mut [u8]) -> Result<(), BusError> {
        let mut msgs = [
            LinuxI2CMessage::write(wr).with_address(u16::from(addr)),
            LinuxI2CMessage::read(rd).with_address(u16::from(addr)),
        ];
        self.transfer(&mut msgs)
    }

    fn recover(&mut self) -> Result<(), BusError> {
        // The kernel driver resets the controller itself on errors.
        Ok(())
    }

    fn info(&self) -> &DeviceInfo {
        &self.info
    }
}
