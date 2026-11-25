//! Zenoh transport: generic pub/sub and request/response for anything declared in `rt_core`
//! (`topics`, `services`). Knows nothing about particular messages or services.
//!
//! On the robot's host, topics flow over iceoryx2 (`rt-ipc`); Zenoh carries what crosses a process's
//! trust or machine boundary: services (commands), sessions, and topics bridged off the host. Uses
//! Zenoh's synchronous API only, so callers need no async runtime.
//!
//! | Module | What |
//! |---|---|
//! | [`keys`] | where everything lives in Zenoh's key space |
//! | [`session`] | opening a session; announcing a client (liveliness) |
//! | [`codec`] | how messages become bytes (`Raw`, `Json`) |
//! | [`topic`] | `NetPublisher`/`NetSubscriber` for any topic |
//! | [`service`] | `Server` with registered handlers; `call` |

pub mod codec;
pub mod keys;
pub mod service;
pub mod session;
pub mod topic;

pub use codec::{Codec, Json, Raw};
pub use keys::{service_key, session_key, session_name, sessions_wildcard, topic_key};
pub use service::{CALL_TIMEOUT, Server, call};
pub use session::{DEFAULT_ENDPOINT, NetConfig, declare_session, open};
pub use topic::{NetPublisher, NetSubscriber};
pub use zenoh::Session;

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

pub(crate) fn zerr(e: impl std::fmt::Display) -> NetError {
    NetError::Zenoh(e.to_string())
}
