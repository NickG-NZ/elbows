//! `robotd` settings: the `[robotd]` table of the robot's `setup.toml`
//! (`hardware/setups/<setup>/setup.toml`). Other tables in that file belong to device drivers.
//!
//! ```toml
//! [robotd]
//! rate_hz = 200
//!
//! [robotd.rt]
//! cpu = 3
//! priority = 80
//! lock_memory = true
//! required = true          # fail rather than run without RT scheduling
//!
//! [robotd.net]
//! listen = ["tcp/127.0.0.1:7447"]   # Zenoh endpoints for services and remote clients;
//!                                   # e.g. "tcp/0.0.0.0:7447" to accept other machines
//! out = ["state", "status"]         # local topics published to Zenoh (any in rt_core::topics)
//! in = ["plan"]                     # Zenoh topics republished locally (e.g. a remote policy)
//!
//! [robotd.plan]
//! max_obs_age_ms = 500
//! max_jump_rad = 0.3
//! q_min = [-3.14, -3.14, -3.14, -3.14, -3.14, -3.14]
//! q_max = [3.14, 3.14, 3.14, 3.14, 3.14, 3.14]
//! ```

use std::path::Path;

use rt_core::msgs::{DOF, Joints, PlanLimits};
use rt_core::rt::RtConfig;
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub rate_hz: f64,
    /// Consecutive overrunning cycles before the loop latches Fault.
    pub max_consecutive_overruns: u32,
    pub rt: RtSection,
    pub plan: PlanSection,
    pub net: NetSection,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct NetSection {
    pub listen: Vec<String>,
    pub out: Vec<String>,
    #[serde(rename = "in")]
    pub inbound: Vec<String>,
}

impl Default for NetSection {
    fn default() -> Self {
        Self {
            listen: vec![rt_node::DEFAULT_ENDPOINT.to_string()],
            out: vec!["state".into(), "status".into()],
            inbound: vec!["plan".into()],
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            rate_hz: 200.0,
            max_consecutive_overruns: 10,
            rt: RtSection::default(),
            plan: PlanSection::default(),
            net: NetSection::default(),
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RtSection {
    pub cpu: Option<usize>,
    pub priority: Option<i32>,
    pub lock_memory: bool,
    pub required: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PlanSection {
    pub max_obs_age_ms: f64,
    pub max_jump_rad: f32,
    pub q_min: Joints,
    pub q_max: Joints,
}

impl Default for PlanSection {
    fn default() -> Self {
        let pi = std::f32::consts::PI;
        Self {
            max_obs_age_ms: 500.0,
            max_jump_rad: 0.3,
            q_min: [-pi; DOF],
            q_max: [pi; DOF],
        }
    }
}

impl Config {
    /// The `[robotd]` table of `setup`, or defaults if there is no file or no table.
    pub fn load(setup: Option<&Path>) -> Result<Self, String> {
        let Some(path) = setup else {
            return Ok(Self::default());
        };
        let path = &rt_devices::setup::setup_file(path);
        #[derive(Deserialize)]
        struct SetupFile {
            robotd: Option<Config>,
        }
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let file: SetupFile =
            toml::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
        let cfg = file.robotd.unwrap_or_default();
        if !(cfg.rate_hz > 0.0 && cfg.rate_hz.is_finite()) {
            return Err(format!(
                "{}: robotd.rate_hz must be positive",
                path.display()
            ));
        }
        Ok(cfg)
    }

    pub fn rt_config(&self) -> RtConfig {
        let r = &self.rt;
        RtConfig {
            cpu: r.cpu,
            priority: r.priority,
            lock_memory: r.lock_memory,
            required: r.required,
        }
    }

    pub fn plan_limits(&self) -> PlanLimits {
        let p = &self.plan;
        PlanLimits {
            q_min: p.q_min,
            q_max: p.q_max,
            max_obs_age_ns: (p.max_obs_age_ms * 1e6) as u64,
            max_jump: p.max_jump_rad,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_robotd_table_and_ignores_device_tables() {
        let path = std::env::temp_dir().join(format!("robotd-config-{}.toml", std::process::id()));
        std::fs::write(
            &path,
            "[arm]\nusb_serial = \"X\"\n[robotd]\nrate_hz = 500\n[robotd.rt]\ncpu = 2\n",
        )
        .unwrap();
        let cfg = Config::load(Some(&path)).unwrap();
        std::fs::remove_file(&path).unwrap();
        assert_eq!(cfg.rate_hz, 500.0);
        assert_eq!(cfg.rt.cpu, Some(2));
        assert_eq!(cfg.plan.max_jump_rad, 0.3);
    }
}
