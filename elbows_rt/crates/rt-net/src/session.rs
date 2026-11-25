//! Opening a Zenoh session, and announcing a client with a liveliness token.

use zenoh::liveliness::LivelinessToken;
use zenoh::{Session, Wait};

use crate::keys::session_key;
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

/// Announce this process as `role` (e.g. "policy") for as long as the token lives. robotd lists
/// live tokens as sessions and notices at once when one goes away.
pub fn declare_session(
    session: &Session,
    robot: &str,
    role: &str,
) -> Result<LivelinessToken, NetError> {
    let key = session_key(robot, role, &session.zid().to_string());
    session.liveliness().declare_token(key).wait().map_err(zerr)
}
