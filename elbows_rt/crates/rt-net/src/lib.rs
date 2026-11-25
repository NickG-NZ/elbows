//! Zenoh transport: generic pub/sub and request/response for anything declared in `rt_core`
//! (`topics`, `services`). Knows nothing about particular messages or services.
//!
//! On the robot's host, topics flow over iceoryx2 (`rt-ipc`); Zenoh carries what crosses a process's
//! trust or machine boundary: services (commands), sessions, and topics bridged off the host. Uses
//! Zenoh's synchronous API only, so callers need no async runtime.
//!
//! ```text
//! elbows/<robot>/topic/<name>           topics, encoded by a Codec (raw bytes by default)
//! elbows/<robot>/srv/<name>             services: JSON request → JSON response
//! elbows/<robot>/session/<role>/<id>    liveliness tokens of connected clients
//! ```

use std::collections::HashMap;
use std::marker::PhantomData;
use std::time::Duration;

use rt_core::services::Service;
use rt_core::topics::Topic;
use rt_core::{Message, type_name};
use serde::Serialize;
use serde::de::DeserializeOwned;
use zenoh::Wait;
use zenoh::handlers::FifoChannelHandler;
use zenoh::liveliness::LivelinessToken;
use zenoh::query::{Query, Queryable};
use zenoh::sample::Sample;

pub use zenoh::Session;

/// Where robotd listens and clients connect, unless configured otherwise.
pub const DEFAULT_ENDPOINT: &str = "tcp/127.0.0.1:7447";
/// How long a client waits for a service reply.
pub const CALL_TIMEOUT: Duration = Duration::from_secs(2);
const ROOT: &str = "elbows";

#[derive(Debug, thiserror::Error)]
pub enum NetError {
    #[error("zenoh: {0}")]
    Zenoh(String),
    #[error("no reply to {key} (is robotd running and reachable at the configured endpoint?)")]
    NoReply { key: String },
    #[error("{key}: {msg}")]
    Remote { key: String, msg: String },
    #[error("decode: {0}")]
    Decode(String),
}

fn zerr(e: impl std::fmt::Display) -> NetError {
    NetError::Zenoh(e.to_string())
}

// ---- Keys ----------------------------------------------------------------------------------------

pub fn topic_key<T>(robot: &str, topic: &Topic<T>) -> String {
    format!("{ROOT}/{robot}/topic/{}", topic.name)
}

pub fn service_key<Req, Resp>(robot: &str, service: &Service<Req, Resp>) -> String {
    format!("{ROOT}/{robot}/srv/{}", service.name)
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

// ---- Codecs --------------------------------------------------------------------------------------

/// How a topic's messages become bytes on the network. Publishers, subscribers and bridges are
/// generic over it, so a topic can switch encoding (e.g. to protobuf, for clients in other
/// languages) without touching anything else.
pub trait Codec<T> {
    fn encode(value: &T) -> Vec<u8>;
    fn decode(bytes: &[u8]) -> Result<T, NetError>;
}

/// The message's own `#[repr(C)]` bytes, prefixed by its versioned type name (`elbows.Plan/1\0`),
/// so a peer built from a different layout is refused rather than misread. Little-endian.
pub struct Raw;

impl<T: Message> Codec<T> for Raw {
    fn encode(value: &T) -> Vec<u8> {
        let name = type_name::<T>().as_bytes();
        let mut out = Vec::with_capacity(name.len() + 1 + size_of::<T>());
        out.extend_from_slice(name);
        out.push(0);
        out.extend_from_slice(bytemuck::bytes_of(value));
        out
    }

    fn decode(bytes: &[u8]) -> Result<T, NetError> {
        let split = bytes
            .iter()
            .position(|&b| b == 0)
            .ok_or(NetError::Decode("no type name".into()))?;
        let (name, body) = (&bytes[..split], &bytes[split + 1..]);
        if name != type_name::<T>().as_bytes() || body.len() != size_of::<T>() {
            return Err(NetError::Decode(format!(
                "got {:?} ({} bytes), expected {:?} ({} bytes)",
                String::from_utf8_lossy(name),
                body.len(),
                type_name::<T>(),
                size_of::<T>()
            )));
        }
        Ok(bytemuck::pod_read_unaligned(body))
    }
}

/// JSON, for services (and any topic that should be readable by anything).
pub struct Json;

impl<T: Serialize + DeserializeOwned> Codec<T> for Json {
    fn encode(value: &T) -> Vec<u8> {
        serde_json::to_vec(value).expect("serializable")
    }

    fn decode(bytes: &[u8]) -> Result<T, NetError> {
        // An empty payload is `null`, i.e. `()` for requests without arguments.
        let bytes = if bytes.is_empty() {
            b"null".as_slice()
        } else {
            bytes
        };
        serde_json::from_slice(bytes).map_err(|e| NetError::Decode(e.to_string()))
    }
}

// ---- Topics --------------------------------------------------------------------------------------

pub struct NetPublisher<T, C = Raw> {
    publisher: zenoh::pubsub::Publisher<'static>,
    _types: PhantomData<fn(T, C)>,
}

impl<T, C: Codec<T>> NetPublisher<T, C> {
    pub fn new(session: &Session, robot: &str, topic: &Topic<T>) -> Result<Self, NetError> {
        let publisher = session
            .declare_publisher(topic_key(robot, topic))
            .wait()
            .map_err(zerr)?;
        Ok(Self {
            publisher,
            _types: PhantomData,
        })
    }

    pub fn publish(&self, value: &T) -> Result<(), NetError> {
        self.publisher.put(C::encode(value)).wait().map_err(zerr)
    }
}

pub struct NetSubscriber<T, C = Raw> {
    subscriber: zenoh::pubsub::Subscriber<FifoChannelHandler<Sample>>,
    _types: PhantomData<fn() -> (T, C)>,
}

impl<T, C: Codec<T>> NetSubscriber<T, C> {
    pub fn new(session: &Session, robot: &str, topic: &Topic<T>) -> Result<Self, NetError> {
        let subscriber = session
            .declare_subscriber(topic_key(robot, topic))
            .wait()
            .map_err(zerr)?;
        Ok(Self {
            subscriber,
            _types: PhantomData,
        })
    }

    /// Every message received since the last call, oldest first; undecodable ones as errors.
    pub fn drain(&self) -> impl Iterator<Item = Result<T, NetError>> + '_ {
        std::iter::from_fn(|| self.subscriber.try_recv().ok().flatten())
            .map(|s| C::decode(&s.payload().to_bytes()))
    }

    /// The newest valid message received since the last call, if any.
    pub fn latest(&self) -> Option<T> {
        self.drain().filter_map(Result::ok).last()
    }
}

// ---- Services ------------------------------------------------------------------------------------

type Handler<Ctx> = Box<dyn FnMut(&mut Ctx, &[u8]) -> Result<Vec<u8>, String> + Send>;

/// Serves the services registered with [`handle`](Server::handle). Handlers get `&mut Ctx`, the
/// state they act on, which the caller passes to [`poll`](Server::poll).
pub struct Server<Ctx> {
    queryable: Queryable<FifoChannelHandler<Query>>,
    prefix: String,
    handlers: HashMap<&'static str, Handler<Ctx>>,
}

impl<Ctx> Server<Ctx> {
    /// Serve `elbows/<robot>/srv/*`.
    pub fn new(session: &Session, robot: &str) -> Result<Self, NetError> {
        let prefix = format!("{ROOT}/{robot}/srv/");
        let queryable = session
            .declare_queryable(format!("{prefix}*"))
            .wait()
            .map_err(zerr)?;
        Ok(Self {
            queryable,
            prefix,
            handlers: HashMap::new(),
        })
    }

    pub fn handle<Req, Resp>(
        &mut self,
        service: &Service<Req, Resp>,
        mut handler: impl FnMut(&mut Ctx, Req) -> Resp + Send + 'static,
    ) -> &mut Self
    where
        Req: Serialize + DeserializeOwned + 'static,
        Resp: Serialize + DeserializeOwned + 'static,
    {
        let erased: Handler<Ctx> = Box::new(move |ctx, bytes| {
            let req =
                <Json as Codec<Req>>::decode(bytes).map_err(|e| format!("bad request: {e}"))?;
            Ok(<Json as Codec<Resp>>::encode(&handler(ctx, req)))
        });
        self.handlers.insert(service.name, erased);
        self
    }

    /// Wait up to `timeout` for a request, then serve it and any others already waiting.
    /// Returns how many were served.
    pub fn poll(&mut self, ctx: &mut Ctx, timeout: Duration) -> usize {
        let Ok(Some(first)) = self.queryable.recv_timeout(timeout) else {
            return 0;
        };
        let mut served = 0;
        let mut next = Some(first);
        while let Some(query) = next {
            self.serve(ctx, &query);
            served += 1;
            next = self.queryable.try_recv().ok().flatten();
        }
        served
    }

    fn serve(&mut self, ctx: &mut Ctx, query: &Query) {
        let key = query.key_expr().as_str();
        let name = key.strip_prefix(self.prefix.as_str()).unwrap_or(key);
        let payload = query
            .payload()
            .map(|p| p.to_bytes().into_owned())
            .unwrap_or_default();
        let result = match self.handlers.get_mut(name) {
            Some(handler) => handler(ctx, &payload),
            None => Err(format!("unknown service {name:?}")),
        };
        let sent = match result {
            Ok(bytes) => query.reply(query.key_expr().clone(), bytes).wait(),
            Err(msg) => query.reply_err(msg).wait(),
        };
        if let Err(e) = sent {
            eprintln!("rt-net: replying to {key}: {e}");
        }
    }
}

/// Call a service on the robotd for `robot` and wait for its response.
pub fn call<Req, Resp>(
    session: &Session,
    robot: &str,
    service: &Service<Req, Resp>,
    request: &Req,
) -> Result<Resp, NetError>
where
    Req: Serialize + DeserializeOwned,
    Resp: Serialize + DeserializeOwned,
{
    let key = service_key(robot, service);
    let replies = session
        .get(&key)
        .payload(<Json as Codec<Req>>::encode(request))
        .timeout(CALL_TIMEOUT)
        .wait()
        .map_err(zerr)?;
    let reply = replies
        .recv()
        .map_err(|_| NetError::NoReply { key: key.clone() })?;
    match reply.into_result() {
        Ok(sample) => <Json as Codec<Resp>>::decode(&sample.payload().to_bytes()),
        Err(e) => Err(NetError::Remote {
            key,
            msg: e.payload().try_to_string().unwrap_or_default().into_owned(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use iceoryx2_bb_derive_macros::ZeroCopySend;
    use iceoryx2_bb_elementary_traits::zero_copy_send::ZeroCopySend;
    use rt_core::services::{ENABLE, HOLD, Reply};

    #[repr(C)]
    #[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable, ZeroCopySend)]
    #[type_name("elbows.test.Pair/1")]
    struct Pair {
        a: u64,
        b: f32,
        c: u32,
    }

    #[repr(C)]
    #[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable, ZeroCopySend)]
    #[type_name("elbows.test.Pair/2")]
    struct PairV2 {
        a: u64,
        c: u32,
        b: f32,
    }

    #[test]
    fn keys() {
        assert_eq!(service_key("so101", &ENABLE), "elbows/so101/srv/enable");
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
    fn raw_codec_round_trips_and_refuses_other_layouts() {
        let v = Pair { a: 7, b: 1.5, c: 3 };
        let bytes = <Raw as Codec<Pair>>::encode(&v);
        assert_eq!(<Raw as Codec<Pair>>::decode(&bytes).unwrap(), v);
        // same size, reordered fields, bumped version: refused, not misread
        assert!(<Raw as Codec<PairV2>>::decode(&bytes).is_err());
        assert!(<Raw as Codec<Pair>>::decode(&bytes[..bytes.len() - 1]).is_err());
        assert!(<Raw as Codec<Pair>>::decode(b"").is_err());
    }

    #[test]
    fn json_codec_handles_empty_requests() {
        assert_eq!(<Json as Codec<()>>::decode(b"").unwrap(), ());
        let reply = Reply {
            ok: true,
            message: "enabled".into(),
            ..Default::default()
        };
        let bytes = <Json as Codec<Reply>>::encode(&reply);
        assert!(bytes.starts_with(br#"{"ok":true,"message":"enabled""#));
        assert_eq!(<Json as Codec<Reply>>::decode(&bytes).unwrap(), reply);
    }

    /// A real round trip through Zenoh on a private localhost port.
    #[test]
    fn server_dispatches_by_service() {
        let endpoint = format!("tcp/127.0.0.1:{}", 17_000 + std::process::id() % 1000);
        let server_session = open(&NetConfig {
            listen: vec![endpoint.clone()],
            connect: Vec::new(),
        })
        .unwrap();
        let client = open(&NetConfig::client(&endpoint)).unwrap();
        let mut server = Server::<u32>::new(&server_session, "t").unwrap();
        server.handle(&ENABLE, |calls: &mut u32, ()| {
            *calls += 1;
            Reply {
                ok: true,
                message: format!("call {calls}"),
                ..Default::default()
            }
        });
        let serving = std::thread::spawn(move || {
            let mut calls = 0;
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            while calls < 2 && std::time::Instant::now() < deadline {
                server.poll(&mut calls, Duration::from_millis(10));
            }
            drop(server_session);
        });
        // the client may need a moment to connect
        let first = (0..50)
            .find_map(|_| call(&client, "t", &ENABLE, &()).ok())
            .unwrap();
        assert_eq!(first.message, "call 1");
        assert!(
            matches!(call(&client, "t", &HOLD, &()), Err(NetError::Remote { .. })),
            "unregistered"
        );
        assert_eq!(call(&client, "t", &ENABLE, &()).unwrap().message, "call 2");
        serving.join().unwrap();
    }
}
