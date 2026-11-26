//! Opening a Zenoh session.

use zenoh::{Session, Wait};

use crate::{NetError, zerr};

/// Where robotd listens and clients connect, unless configured otherwise.
pub const DEFAULT_ENDPOINT: &str = "tcp/127.0.0.1:7447";

/// Where to listen and what to connect to. Multicast discovery is off: endpoints are explicit.
#[derive(Debug, Clone, Default)]
pub struct NetConfig {
    pub listen: Vec<String>,
    pub connect: Vec<String>,
}

impl NetConfig {
    /// A client of the robotd at `endpoint`.
    pub fn client(endpoint: &str) -> Self {
        Self {
            listen: Vec::new(),
            connect: vec![endpoint.to_string()],
        }
    }

    /// Accept connections on `endpoints` (robotd).
    pub fn server(endpoints: &[String]) -> Self {
        Self {
            listen: endpoints.to_vec(),
            connect: Vec::new(),
        }
    }
}

pub fn open(cfg: &NetConfig) -> Result<Session, NetError> {
    let mut config = zenoh::Config::default();
    let json = |v: &[String]| serde_json::to_string(v).expect("strings serialize");
    for (key, value) in [
        ("mode", "\"peer\"".to_string()),
        ("scouting/multicast/enabled", "false".to_string()),
        ("listen/endpoints", json(&cfg.listen)),
        ("connect/endpoints", json(&cfg.connect)),
    ] {
        config
            .insert_json5(key, &value)
            .map_err(|e| NetError::Zenoh(format!("config {key}: {e}")))?;
    }
    zenoh::open(config).wait().map_err(zerr)
}
