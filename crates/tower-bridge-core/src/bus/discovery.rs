//! Finding and opening TOWER Bridge Modules.

use std::ffi::CString;

use hidapi::HidApi;
use serde::{Deserialize, Serialize};
use tracing::{debug, info};

use super::I2cBus;
use super::ft260::{self, Ft260, Timing};
use crate::model::{Backend, DeviceErrorKind, DeviceInfo};

/// Which transport to use.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackendPref {
    /// HID first, then (Linux) the kernel I2C adapter.
    #[default]
    Auto,
    Hid,
    #[serde(alias = "i2c-dev")]
    I2cDev,
}

/// A bridge found during a scan.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct Candidate {
    pub backend: Backend,
    pub path: String,
    pub serial: Option<String>,
    /// USB interface number (-1 when unknown).
    pub interface: i32,
    pub usage_page: u16,
    pub product: Option<String>,
    /// False when the HID interface is not the I2C one, or the `/dev/i2c-N`
    /// node is missing.
    pub usable: bool,
}

/// Failure to open a bridge.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct OpenError {
    pub kind: DeviceErrorKind,
    pub message: String,
}

impl OpenError {
    fn new(kind: DeviceErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }
}

fn is_i2c_interface(info: &hidapi::DeviceInfo) -> bool {
    if info.interface_number() == ft260::I2C_INTERFACE {
        return true;
    }
    let path = info.path().to_string_lossy().to_ascii_lowercase();
    if path.contains("&mi_00") {
        return true;
    }
    info.interface_number() < 0 && !path.contains("&mi_01")
}

fn classify_hid_error(message: &str) -> DeviceErrorKind {
    let m = message.to_ascii_lowercase();
    if m.contains("permission") || m.contains("access denied") || m.contains("access is denied") {
        DeviceErrorKind::PermissionDenied
    } else if m.contains("exclusive") || m.contains("busy") {
        DeviceErrorKind::Busy
    } else {
        DeviceErrorKind::Failed
    }
}

/// Keeps the hidapi context alive between scans.
pub struct Discovery {
    api: Option<HidApi>,
    pref: BackendPref,
    timing: Timing,
}

impl Discovery {
    pub fn new(pref: BackendPref) -> Self {
        Self {
            api: None,
            pref,
            timing: Timing::DEFAULT,
        }
    }

    pub fn set_preference(&mut self, pref: BackendPref) {
        self.pref = pref;
    }

    fn api(&mut self) -> Result<&mut HidApi, OpenError> {
        if self.api.is_none() {
            let api = HidApi::new().map_err(|e| {
                OpenError::new(DeviceErrorKind::Failed, format!("HID init failed: {e}"))
            })?;
            self.api = Some(api);
        }
        self.api
            .as_mut()
            .ok_or_else(|| OpenError::new(DeviceErrorKind::Failed, "HID init failed"))
    }

    /// List HID bridges (and, on Linux, kernel I2C adapters).
    pub fn scan(&mut self) -> Result<Vec<Candidate>, OpenError> {
        let mut out = Vec::new();
        if self.pref != BackendPref::I2cDev {
            let api = self.api()?;
            api.reset_devices()
                .and_then(|_| api.add_devices(ft260::VID, ft260::PID))
                .map_err(|e| OpenError::new(DeviceErrorKind::Failed, e.to_string()))?;
            for d in api.device_list() {
                out.push(Candidate {
                    backend: Backend::Hid,
                    path: d.path().to_string_lossy().into_owned(),
                    serial: d
                        .serial_number()
                        .map(str::to_owned)
                        .filter(|s| !s.is_empty()),
                    interface: d.interface_number(),
                    usage_page: d.usage_page(),
                    product: d.product_string().map(str::to_owned),
                    usable: is_i2c_interface(d),
                });
            }
        }
        #[cfg(target_os = "linux")]
        if self.pref != BackendPref::Hid {
            for a in super::linux_i2cdev::list_adapters() {
                out.push(Candidate {
                    backend: Backend::I2cDev,
                    path: a.dev_path.display().to_string(),
                    serial: a.serial.clone(),
                    interface: -1,
                    usage_page: 0,
                    product: Some(a.name.clone()),
                    usable: a.dev_node_exists(),
                });
            }
        }
        Ok(out)
    }

    /// Open and initialise the first usable bridge.
    pub fn open_first(&mut self) -> Result<Box<dyn I2cBus>, OpenError> {
        let candidates = self.scan()?;
        let mut last_err: Option<OpenError> = None;

        for c in candidates
            .iter()
            .filter(|c| c.backend == Backend::Hid && c.usable)
        {
            match self.open_hid(c) {
                Ok(bus) => return Ok(bus),
                Err(e) => {
                    debug!("cannot open {}: {e}", c.path);
                    last_err = Some(e);
                }
            }
        }

        #[cfg(target_os = "linux")]
        for c in candidates.iter().filter(|c| c.backend == Backend::I2cDev) {
            if !c.usable {
                last_err = Some(OpenError::new(
                    DeviceErrorKind::I2cDevNotLoaded,
                    format!(
                        "The kernel ft260 driver owns the bridge but {} does not exist (load the i2c-dev module)",
                        c.path
                    ),
                ));
                continue;
            }
            let adapter = super::linux_i2cdev::Adapter {
                dev_path: c.path.clone().into(),
                name: c.product.clone().unwrap_or_default(),
                serial: c.serial.clone(),
            };
            match super::linux_i2cdev::LinuxI2cDev::open(&adapter) {
                Ok(bus) => {
                    info!("opened {} (kernel i2c-dev)", c.path);
                    return Ok(Box::new(bus));
                }
                Err(e) => {
                    let kind = if e.kind() == std::io::ErrorKind::PermissionDenied {
                        DeviceErrorKind::PermissionDenied
                    } else {
                        DeviceErrorKind::Failed
                    };
                    last_err = Some(OpenError::new(kind, format!("{}: {e}", c.path)));
                }
            }
        }

        Err(last_err.unwrap_or_else(|| {
            OpenError::new(DeviceErrorKind::NotFound, "No TOWER Bridge Module found")
        }))
    }

    fn open_hid(&mut self, c: &Candidate) -> Result<Box<dyn I2cBus>, OpenError> {
        let timing = self.timing;
        let api = self.api()?;
        let path = CString::new(c.path.clone())
            .map_err(|_| OpenError::new(DeviceErrorKind::Failed, "invalid device path"))?;
        let dev = api.open_path(&path).map_err(|e| {
            let msg = e.to_string();
            OpenError::new(classify_hid_error(&msg), format!("{}: {msg}", c.path))
        })?;
        let info = DeviceInfo {
            backend: Backend::Hid,
            serial: c.serial.clone().unwrap_or_else(|| "ft260".into()),
            path: c.path.clone(),
            chip: None,
        };
        let mut bus = Ft260::new(dev, info).with_timing(timing);
        bus.initialize().map_err(|e| {
            OpenError::new(
                DeviceErrorKind::Failed,
                format!("bridge initialisation failed: {e}"),
            )
        })?;
        info!("opened {} (serial {})", c.path, bus.info().serial);
        Ok(Box::new(bus))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hid_error_classification() {
        assert_eq!(
            classify_hid_error("hid_open_path: Permission denied"),
            DeviceErrorKind::PermissionDenied
        );
        assert_eq!(
            classify_hid_error("(iokit/common) exclusive access and device already open"),
            DeviceErrorKind::Busy
        );
        assert_eq!(classify_hid_error("whatever"), DeviceErrorKind::Failed);
    }
}
