//! Robot setups: one directory per physical rig, e.g. `hardware/setups/so101/` (local, not committed;
//! template `hardware/setups/example/`). Nothing assumes a default setup, so the same code runs any
//! robot given its directory.
//!
//! ```text
//! hardware/setups/<setup>/
//!   setup.toml                     devices on this rig
//!   calibration/<device>/<kind>.toml   this rig's in-situ calibration (see `calibration`)
//! hardware/devices/<serial>/<kind>.toml  per-unit characterization, follows the hardware
//! ```
//!
//! `setup.toml` has one table per device on the rig, keyed by a name you choose. Its keys are
//! whatever that device's driver needs, e.g. a USB serial number for a servo bus. `[robotd]` is
//! reserved for robotd's own settings.
//!
//! ```toml
//! [arm]
//! usb_serial = "5C93150591"
//! ```

use std::path::{Path, PathBuf};

pub const SETUP_FILE: &str = "setup.toml";

/// `setup.toml` inside a setup directory; a path to the file itself is accepted as is.
pub fn setup_file(path: impl AsRef<Path>) -> PathBuf {
    let path = path.as_ref();
    if path.is_dir() {
        path.join(SETUP_FILE)
    } else {
        path.to_path_buf()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SetupError {
    #[error("no setup file {0}; copy hardware/setups/example/ and fill it in")]
    Missing(PathBuf),
    #[error("{path}: {msg}")]
    Invalid { path: PathBuf, msg: String },
    #[error("no [{device}] in {path}; have {have:?}")]
    NoDevice {
        device: String,
        path: PathBuf,
        have: Vec<String>,
    },
}

#[derive(Debug, Clone)]
pub struct Setup {
    path: PathBuf,
    dir: PathBuf,
    tables: toml::Table,
}

impl Setup {
    /// Load a setup from its directory (or its `setup.toml`).
    pub fn load(path: impl AsRef<Path>) -> Result<Self, SetupError> {
        let path = setup_file(path);
        let text = std::fs::read_to_string(&path).map_err(|_| SetupError::Missing(path.clone()))?;
        let tables: toml::Table =
            text.parse()
                .map_err(|e: toml::de::Error| SetupError::Invalid {
                    path: path.clone(),
                    msg: e.to_string(),
                })?;
        if let Some((name, _)) = tables.iter().find(|(_, v)| !v.is_table()) {
            let msg = format!("top-level key {name:?} must be a [table], one per device");
            return Err(SetupError::Invalid { path, msg });
        }
        let dir = path.parent().map_or_else(PathBuf::new, Path::to_path_buf);
        Ok(Self { path, dir, tables })
    }

    /// The `setup.toml` this was loaded from.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The setup directory.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Where this rig's in-situ calibration of `kind` for `device` lives (may not exist yet).
    pub fn calibration_path(&self, device: &str, kind: &str) -> PathBuf {
        self.dir
            .join("calibration")
            .join(device)
            .join(format!("{kind}.toml"))
    }

    /// Where the characterization of `kind` for the unit with this serial lives (may not exist
    /// yet): `hardware/devices/<serial>/<kind>.toml`, two levels above the setup directory.
    pub fn characterization_path(&self, serial: &str, kind: &str) -> PathBuf {
        let hardware = self
            .dir
            .parent()
            .and_then(Path::parent)
            .unwrap_or(Path::new("."));
        hardware
            .join("devices")
            .join(serial)
            .join(format!("{kind}.toml"))
    }

    /// A device's table.
    pub fn device(&self, name: &str) -> Result<&toml::Table, SetupError> {
        self.tables
            .get(name)
            .and_then(|v| v.as_table())
            .ok_or_else(|| SetupError::NoDevice {
                device: name.into(),
                path: self.path.clone(),
                have: self.tables.keys().cloned().collect(),
            })
    }

    /// A string key of a device's table, e.g. its `usb_serial`.
    pub fn device_str(&self, name: &str, key: &str) -> Result<&str, SetupError> {
        self.device(name)?
            .get(key)
            .and_then(|v| v.as_str())
            .ok_or_else(|| SetupError::Invalid {
                path: self.path.clone(),
                msg: format!("[{name}] has no string {key:?}"),
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A setup directory `<tmp>/hardware/setups/<name>/` holding `setup.toml`.
    fn write(name: &str, text: &str) -> PathBuf {
        let dir = std::env::temp_dir()
            .join(format!("rt-devices-setup-{}", std::process::id()))
            .join("hardware/setups")
            .join(name);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(SETUP_FILE), text).unwrap();
        dir
    }

    #[test]
    fn loads_by_directory_or_file_and_derives_calibration_paths() {
        let dir = write("desk", "[arm]\nusb_serial = \"A\"\n");
        let by_dir = Setup::load(&dir).unwrap();
        let by_file = Setup::load(dir.join(SETUP_FILE)).unwrap();
        assert_eq!(by_dir.dir(), by_file.dir());
        assert_eq!(
            by_dir.calibration_path("arm", "joint_ranges"),
            dir.join("calibration/arm/joint_ranges.toml")
        );
        let hardware = dir.parent().unwrap().parent().unwrap();
        assert_eq!(
            by_dir.characterization_path("A", "motor"),
            hardware.join("devices/A/motor.toml")
        );
    }

    #[test]
    fn each_setup_is_its_own_file() {
        let a = Setup::load(write("desk", "[arm]\nusb_serial = \"A\"\n")).unwrap();
        let b = Setup::load(write(
            "lab",
            "[arm]\nusb_serial = \"B\"\n[imu]\nbus = 1\naddress = 0x68\n",
        ))
        .unwrap();
        assert_eq!(a.device_str("arm", "usb_serial").unwrap(), "A");
        assert_eq!(b.device_str("arm", "usb_serial").unwrap(), "B");
        assert_eq!(b.device("imu").unwrap()["address"].as_integer(), Some(0x68));
        assert!(matches!(a.device("imu"), Err(SetupError::NoDevice { .. })));
        assert!(matches!(
            b.device_str("imu", "usb_serial"),
            Err(SetupError::Invalid { .. })
        ));
    }

    #[test]
    fn rejects_missing_and_malformed_files() {
        assert!(matches!(
            Setup::load("/nonexistent/setup.toml"),
            Err(SetupError::Missing(_))
        ));
        assert!(matches!(
            Setup::load(write("flat", "usb_serial = \"A\"\n")),
            Err(SetupError::Invalid { .. })
        ));
        assert!(matches!(
            Setup::load(write("broken", "[arm\n")),
            Err(SetupError::Invalid { .. })
        ));
    }
}
