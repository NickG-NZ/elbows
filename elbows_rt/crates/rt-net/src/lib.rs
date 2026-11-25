//! Zenoh: robotd's control plane (commands, sessions) and topics that leave the host.
//!
//! On the robot's host, data flows over iceoryx2 (`rt-ipc`); Zenoh carries what crosses a process's
//! trust or machine boundary. Uses Zenoh's synchronous API only, so callers need no async runtime.
//!
//! ```text
//! elbows/<robot>/ctl/<command>          queryables served by robotd: status, enable, hold, stop
//! elbows/<robot>/topic/<name>           bridged topics: raw #[repr(C)] bytes, type name attached
//! elbows/<robot>/session/<role>/<id>    liveliness tokens of connected clients (e.g. policies)
//! ```
//!
//! Commands and replies are JSON, so the shell and Python can use them too.

use std::marker::PhantomData;
use std::time::Duration;

use bytemuck::Pod;
use iceoryx2_bb_elementary_traits::zero_copy_send::ZeroCopySend;
use rt_core::topics::Topic;
use serde::{Deserialize, Serialize};
use zenoh::Wait;
use zenoh::handlers::FifoChannelHandler;
use zenoh::liveliness::LivelinessToken;
use zenoh::sample::Sample;

pub use zenoh::Session;

/// Where robotd listens and clients connect, unless configured otherwise.
pub const DEFAULT_ENDPOINT: &str = "tcp/127.0.0.1:7447";
/// How long a client waits for robotd to answer a command.
pub const COMMAND_TIMEOUT: Duration = Duration::from_secs(2);
const ROOT: &str = "elbows";

#[derive(Debug, thiserror::Error)]
pub enum NetError {
    #[error("zenoh: {0}")]
    Zenoh(String),
    #[error(
        "no reply from robotd for {robot} (is it running and reachable at the configured endpoint?)"
    )]
    NoReply { robot: String },
    #[error("bad reply: {0}")]
    BadReply(String),
    #[error(
        "message type mismatch: got {got:?} ({got_len} bytes), expected {expected:?} ({expected_len} bytes)"
    )]
    TypeMismatch {
        got: String,
        got_len: usize,
        expected: &'static str,
        expected_len: usize,
    },
}

fn zerr(e: impl std::fmt::Display) -> NetError {
    NetError::Zenoh(e.to_string())
}

// ---- Keys ----------------------------------------------------------------------------------------

pub fn ctl_key(robot: &str, command: Command) -> String {
    format!("{ROOT}/{robot}/ctl/{}", command.name())
}

pub fn ctl_wildcard(robot: &str) -> String {
    format!("{ROOT}/{robot}/ctl/*")
}

pub fn topic_key<T>(robot: &str, topic: &Topic<T>) -> String {
    format!("{ROOT}/{robot}/topic/{}", topic.name)
}

pub fn session_key(robot: &str, role: &str, id: &str) -> String {
    format!("{ROOT}/{robot}/session/{role}/{id}")
}

pub fn sessions_wildcard(robot: &str) -> String {
    format!("{ROOT}/{robot}/session/**")
}

/// `<role>/<id>` from a session key, for display.
pub fn session_name(key: &str) -> &str {
    key.split("/session/").nth(1).unwrap_or(key)
}

// ---- Sessions ------------------------------------------------------------------------------------

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

/// Announce this process as `role` (e.g. "policy") for as long as the token lives.
pub fn declare_session(
    session: &Session,
    robot: &str,
    role: &str,
) -> Result<LivelinessToken, NetError> {
    let key = session_key(robot, role, &session.zid().to_string());
    session.liveliness().declare_token(key).wait().map_err(zerr)
}

// ---- Commands ------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Command {
    Status,
    Enable,
    Hold,
    Stop,
}

impl Command {
    pub const ALL: [Command; 4] = [
        Command::Status,
        Command::Enable,
        Command::Hold,
        Command::Stop,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Command::Status => "status",
            Command::Enable => "enable",
            Command::Hold => "hold",
            Command::Stop => "stop",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|c| c.name() == s)
    }

    /// The command a `ctl/<command>` key addresses.
    pub fn from_key(key: &str) -> Option<Self> {
        Self::parse(key.rsplit('/').next()?)
    }
}

/// robotd's answer to every command.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Reply {
    /// Whether the command was carried out (`status` always is).
    pub ok: bool,
    /// Why not, or what happened.
    pub message: String,
    pub status: StatusReport,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct StatusReport {
    pub mode: String,
    pub plan_seq: u64,
    pub plans_rejected: u32,
    pub cycles: u64,
    pub missed: u64,
    pub latency_p99_us: u64,
    pub latency_max_us: u64,
    pub exec_p99_us: u64,
    /// Connected clients, as `<role>/<id>`.
    pub sessions: Vec<String>,
}

/// Send `command` to the robotd for `robot` and wait for its reply.
pub fn request(session: &Session, robot: &str, command: Command) -> Result<Reply, NetError> {
    let replies = session
        .get(ctl_key(robot, command))
        .timeout(COMMAND_TIMEOUT)
        .wait()
        .map_err(zerr)?;
    let reply = replies.recv().map_err(|_| NetError::NoReply {
        robot: robot.into(),
    })?;
    let sample = reply
        .into_result()
        .map_err(|e| NetError::BadReply(format!("{:?}", e.payload().try_to_string())))?;
    serde_json::from_slice(&sample.payload().to_bytes())
        .map_err(|e| NetError::BadReply(e.to_string()))
}

// ---- Topics --------------------------------------------------------------------------------------

/// A message that can cross the network: plain data with a stable type name. The same types as
/// `rt-ipc` topics, sent as raw little-endian bytes.
pub trait WireMessage: Pod + ZeroCopySend {}
impl<T: Pod + ZeroCopySend> WireMessage for T {}

fn type_name<T: WireMessage>() -> &'static str {
    // SAFETY: only reads the type's declared name.
    unsafe { <T as ZeroCopySend>::type_name() }
}

pub fn encode<T: WireMessage>(value: &T) -> (Vec<u8>, &'static str) {
    (bytemuck::bytes_of(value).to_vec(), type_name::<T>())
}

pub fn decode<T: WireMessage>(payload: &[u8], attachment: Option<&[u8]>) -> Result<T, NetError> {
    let got = attachment.map(String::from_utf8_lossy).unwrap_or_default();
    if got != type_name::<T>() || payload.len() != size_of::<T>() {
        return Err(NetError::TypeMismatch {
            got: got.into_owned(),
            got_len: payload.len(),
            expected: type_name::<T>(),
            expected_len: size_of::<T>(),
        });
    }
    Ok(bytemuck::pod_read_unaligned(payload))
}

pub struct NetPublisher<T> {
    publisher: zenoh::pubsub::Publisher<'static>,
    _msg: PhantomData<fn(T)>,
}

impl<T: WireMessage> NetPublisher<T> {
    pub fn new(session: &Session, robot: &str, topic: &Topic<T>) -> Result<Self, NetError> {
        let publisher = session
            .declare_publisher(topic_key(robot, topic))
            .wait()
            .map_err(zerr)?;
        Ok(Self {
            publisher,
            _msg: PhantomData,
        })
    }

    pub fn publish(&self, value: &T) -> Result<(), NetError> {
        let (bytes, name) = encode(value);
        self.publisher
            .put(bytes)
            .attachment(name)
            .wait()
            .map_err(zerr)
    }
}

pub struct NetSubscriber<T> {
    subscriber: zenoh::pubsub::Subscriber<FifoChannelHandler<Sample>>,
    _msg: PhantomData<fn() -> T>,
}

impl<T: WireMessage> NetSubscriber<T> {
    pub fn new(session: &Session, robot: &str, topic: &Topic<T>) -> Result<Self, NetError> {
        let subscriber = session
            .declare_subscriber(topic_key(robot, topic))
            .wait()
            .map_err(zerr)?;
        Ok(Self {
            subscriber,
            _msg: PhantomData,
        })
    }

    /// Every message received since the last call, oldest first; undecodable ones are returned as
    /// errors in place.
    pub fn drain(&self) -> impl Iterator<Item = Result<T, NetError>> + '_ {
        std::iter::from_fn(|| self.subscriber.try_recv().ok().flatten()).map(|s| {
            let attachment = s.attachment().map(|a| a.to_bytes().into_owned());
            decode(&s.payload().to_bytes(), attachment.as_deref())
        })
    }

    /// The newest valid message received since the last call, if any.
    pub fn latest(&self) -> Option<T> {
        self.drain().filter_map(Result::ok).last()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use iceoryx2_bb_derive_macros::ZeroCopySend;

    #[repr(C)]
    #[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable, ZeroCopySend)]
    #[type_name("elbows.test.Pair")]
    struct Pair {
        a: u64,
        b: f32,
        c: u32,
    }

    #[repr(C)]
    #[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable, ZeroCopySend)]
    #[type_name("elbows.test.Other")]
    struct Other {
        a: u64,
        b: f32,
        c: u32,
    }

    #[test]
    fn keys() {
        assert_eq!(ctl_key("so101", Command::Enable), "elbows/so101/ctl/enable");
        assert_eq!(
            Command::from_key("elbows/so101/ctl/stop"),
            Some(Command::Stop)
        );
        assert_eq!(Command::from_key("elbows/so101/ctl/fly"), None);
        assert_eq!(
            topic_key("so101", &rt_core::topics::PLAN),
            "elbows/so101/topic/plan"
        );
        assert_eq!(
            session_name("elbows/so101/session/policy/abc"),
            "policy/abc"
        );
    }

    #[test]
    fn wire_round_trip_and_type_check() {
        let v = Pair { a: 7, b: 1.5, c: 3 };
        let (bytes, name) = encode(&v);
        assert_eq!(decode::<Pair>(&bytes, Some(name.as_bytes())).unwrap(), v);
        // same size and layout, different type: refused
        assert!(matches!(
            decode::<Other>(&bytes, Some(name.as_bytes())),
            Err(NetError::TypeMismatch { .. })
        ));
        assert!(matches!(
            decode::<Pair>(&bytes[1..], Some(name.as_bytes())),
            Err(NetError::TypeMismatch { .. })
        ));
        assert!(matches!(
            decode::<Pair>(&bytes, None),
            Err(NetError::TypeMismatch { .. })
        ));
    }

    #[test]
    fn reply_is_plain_json() {
        let reply = Reply {
            ok: true,
            message: "enabled".into(),
            status: StatusReport::default(),
        };
        let json = serde_json::to_string(&reply).unwrap();
        assert!(json.starts_with(r#"{"ok":true,"message":"enabled","status":{"mode":"""#));
        assert_eq!(serde_json::from_str::<Reply>(&json).unwrap(), reply);
    }
}
