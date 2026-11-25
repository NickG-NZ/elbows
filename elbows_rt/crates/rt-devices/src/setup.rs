//! Robot setup files: one TOML per physical rig, e.g. `hardware/setups/so101.toml` (local, not
//! committed; template `hardware/setups/example.toml`). Nothing assumes a default setup, so the same
//! code runs any robot given its file.
//!
//! One table per device on the rig, keyed by a name you choose. Its keys are whatever that device's
//! driver needs, e.g. a USB serial number for a servo bus. `[robotd]` is reserved for robotd's own
//! settings.
//!
//! ```toml
//! [arm]
//! usb_serial = "5C93150591"
//! ```

use std::path::{Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum SetupError {
    #[error("no setup file {0}; copy hardware/setups/example.toml and fill it in")]
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
    tables: toml::Table,
}

impl Setup {
    pub fn load(path: impl AsRef<Path>) -> Result<Self, SetupError> {
        let path = path.as_ref().to_path_buf();
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
        Ok(Self { path, tables })
    }

    pub fn path(&self) -> &Path {
        &self.path
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

    fn write(name: &str, text: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("rt-devices-setup-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);
        std::fs::write(&path, text).unwrap();
        path
    }

    #[test]
    fn each_setup_is_its_own_file() {
        let a = Setup::load(write("desk.toml", "[arm]\nusb_serial = \"A\"\n")).unwrap();
        let b = Setup::load(write(
            "lab.toml",
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
            Setup::load(write("flat.toml", "usb_serial = \"A\"\n")),
            Err(SetupError::Invalid { .. })
        ));
        assert!(matches!(
            Setup::load(write("broken.toml", "[arm\n")),
            Err(SetupError::Invalid { .. })
        ));
    }
}
