//! Zenoh transport: generic pub/sub and request/response for anything declared in `rt_core`
//! (`topics`, `services`). Knows nothing about particular messages or services.
//!
//! A backend of `rt-node`: applications use `rt_node::Node`, not this crate. On the robot's host,
//! topics flow over iceoryx2 (`rt-ipc`); Zenoh carries services, node presence, and topics to and
//! from other machines. Uses Zenoh's synchronous API only, so callers need no async runtime.
//!
//! | Module | What |
//! |---|---|
//! | [`keys`] | where everything lives in Zenoh's key space |
//! | [`session`] | opening a session |
//! | [`presence`] | announcing a node; watching which nodes are present |
//! | [`codec`] | how messages become bytes (`Raw`, `Json`) |
//! | [`pubsub`] | `NetPublisher`/`NetSubscriber` for any topic |
//! | [`server`] | `Server`: serve services with registered handlers |
//! | [`client`] | `call` a service |

pub mod client;
pub mod codec;
pub mod keys;
pub mod presence;
pub mod pubsub;
pub mod server;
pub mod session;

pub use client::{CALL_TIMEOUT, call};
pub use codec::{Codec, Json, Raw};
pub use presence::{NodeEvent, NodeWatch, announce};
pub use pubsub::{NetPublisher, NetSubscriber};
pub use server::Server;
pub use session::{DEFAULT_ENDPOINT, NetConfig, open};
pub use zenoh::Session;
pub use zenoh::liveliness::LivelinessToken;

#[derive(Debug, thiserror::Error)]
pub enum NetError {
    #[error("zenoh: {0}")]
    Zenoh(String),
    #[error("no reply to {key} (is its server running and reachable at the configured endpoint?)")]
    NoReply { key: String },
    #[error("{key}: {msg}")]
    Remote { key: String, msg: String },
    #[error("{key} is already served by another node")]
    Owned { key: String },
    #[error("decode: {0}")]
    Decode(String),
}

pub(crate) fn zerr(e: impl std::fmt::Display) -> NetError {
    NetError::Zenoh(e.to_string())
}
