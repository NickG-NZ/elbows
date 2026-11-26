//! Which nodes are on the network: each announces itself with a liveliness token, which Zenoh
//! withdraws when the node exits, crashes or loses its connection.

use zenoh::handlers::FifoChannelHandler;
use zenoh::liveliness::LivelinessToken;
use zenoh::sample::{Sample, SampleKind};
use zenoh::{Session, Wait};

use crate::keys::{node_key, node_label, nodes_wildcard};
use crate::{NetError, zerr};

/// Announce `name` on the network for as long as the token lives.
pub fn announce(session: &Session, robot: &str, name: &str) -> Result<LivelinessToken, NetError> {
    let key = node_key(robot, name, &session.zid().to_string());
    session.liveliness().declare_token(key).wait().map_err(zerr)
}

/// A node appearing or going away.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeEvent {
    /// `<name>/<id>`.
    pub label: String,
    pub alive: bool,
}

/// Watches the robot's nodes. The first events are the nodes already present.
pub struct NodeWatch {
    subscriber: zenoh::pubsub::Subscriber<FifoChannelHandler<Sample>>,
}

impl NodeWatch {
    pub fn new(session: &Session, robot: &str) -> Result<Self, NetError> {
        let subscriber = session
            .liveliness()
            .declare_subscriber(nodes_wildcard(robot))
            .history(true)
            .wait()
            .map_err(zerr)?;
        Ok(Self { subscriber })
    }

    /// Every event since the last call, oldest first.
    pub fn drain(&self) -> impl Iterator<Item = NodeEvent> + '_ {
        std::iter::from_fn(|| self.subscriber.try_recv().ok().flatten()).map(|s| NodeEvent {
            label: node_label(s.key_expr().as_str()).to_string(),
            alive: s.kind() == SampleKind::Put,
        })
    }
}
