//! Calibration files: the contract between calibration procedures (Python, in `hardware/`) and the
//! drivers that use the results. Calibration data is local, never committed.
//!
//! Two kinds, stored by what they're tied to (paths from [`Setup`](crate::setup::Setup)):
//! - **in-situ**, tied to one rig (joint offsets and ranges, camera mounting pose):
//!   `hardware/setups/<setup>/calibration/<device>/<kind>.toml`
//! - **characterization**, tied to one physical unit (camera intrinsics, motor parameters):
//!   `hardware/devices/<serial>/<kind>.toml`
//!
//! Every file is a `[calibration]` header plus a `[data]` table whose layout the `kind` defines:
//!
//! ```toml
//! [calibration]
//! kind = "joint_ranges"
//! version = 1
//! device = "arm"
//! serial = "5C93150591"           # optional
//! created = 2026-11-25T10:00:00Z
//! method = "manual range sweep"   # optional
//! [calibration.quality]           # optional
//! [data]
//! ```
//!
//! A driver states the kind and the version of the `[data]` layout it understands; any other kind or
//! version is refused rather than misread.

use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde::de::DeserializeOwned;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Header {
    pub kind: String,
    pub version: u32,
    pub device: String,
    #[serde(default)]
    pub serial: Option<String>,
    pub created: toml::value::Datetime,
    #[serde(default)]
    pub method: String,
    #[serde(default)]
    pub quality: toml::Table,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Calibration {
    #[serde(rename = "calibration")]
    pub header: Header,
    pub data: toml::Table,
}

#[derive(Debug, thiserror::Error)]
pub enum CalibrationError {
    #[error("no calibration file {0}; run its calibration procedure (see hardware/README.md)")]
    Missing(PathBuf),
    #[error("{path}: {msg}")]
    Invalid { path: PathBuf, msg: String },
    #[error("{path}: holds {found:?} calibration, expected {expected:?}")]
    Kind {
        path: PathBuf,
        expected: String,
        found: String,
    },
    #[error(
        "{path}: {kind} version {found}, this build understands version {supported}; recalibrate"
    )]
    Version {
        path: PathBuf,
        kind: String,
        found: u32,
        supported: u32,
    },
}

impl Calibration {
    /// Load a calibration of `kind`, refusing any `[data]` layout version but `version`.
    pub fn load(
        path: impl AsRef<Path>,
        kind: &str,
        version: u32,
    ) -> Result<Self, CalibrationError> {
        let path = path.as_ref();
        let invalid = |msg: String| CalibrationError::Invalid {
            path: path.into(),
            msg,
        };
        let text =
            std::fs::read_to_string(path).map_err(|_| CalibrationError::Missing(path.into()))?;
        let cal: Calibration = toml::from_str(&text).map_err(|e| invalid(e.to_string()))?;
        let h = &cal.header;
        if h.kind != kind {
            return Err(CalibrationError::Kind {
                path: path.into(),
                expected: kind.into(),
                found: h.kind.clone(),
            });
        }
        if h.version != version {
            return Err(CalibrationError::Version {
                path: path.into(),
                kind: kind.into(),
                found: h.version,
                supported: version,
            });
        }
        Ok(cal)
    }

    /// The `[data]` table as a typed struct defined by the driver that uses this kind.
    pub fn data_as<T: DeserializeOwned>(&self) -> Result<T, toml::de::Error> {
        toml::Value::Table(self.data.clone()).try_into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Shared with the Python tests, so both readers stay in sync.
    const EXAMPLE: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../hardware/tests/fixtures/example_calibration.toml"
    );

    #[derive(Deserialize)]
    struct ExampleData {
        joints: Vec<String>,
        offset: Vec<i64>,
    }

    #[test]
    fn reads_the_shared_example() {
        let cal = Calibration::load(EXAMPLE, "example", 1).unwrap();
        assert_eq!(cal.header.device, "arm");
        assert_eq!(cal.header.serial.as_deref(), Some("5C93150591"));
        assert_eq!(cal.header.quality["residual_rms"].as_float(), Some(0.5));
        let data: ExampleData = cal.data_as().unwrap();
        assert_eq!(data.joints.len(), data.offset.len());
        assert_eq!(data.joints[0], "shoulder_pan");
    }

    #[test]
    fn refuses_wrong_kind_version_or_missing_file() {
        assert!(matches!(
            Calibration::load(EXAMPLE, "joint_ranges", 1),
            Err(CalibrationError::Kind { .. })
        ));
        assert!(matches!(
            Calibration::load(EXAMPLE, "example", 2),
            Err(CalibrationError::Version { .. })
        ));
        assert!(matches!(
            Calibration::load("/nonexistent.toml", "example", 1),
            Err(CalibrationError::Missing(_))
        ));
    }

    #[test]
    fn refuses_a_malformed_header() {
        let path = std::env::temp_dir().join(format!("rt-devices-cal-{}.toml", std::process::id()));
        std::fs::write(
            &path,
            "[calibration]\nkind = \"example\"\nversion = 1\n[data]\n",
        )
        .unwrap();
        assert!(matches!(
            Calibration::load(&path, "example", 1),
            Err(CalibrationError::Invalid { .. })
        ));
    }
}
