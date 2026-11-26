//! A [`Node`] is one participant in a robot's messaging: it publishes and subscribes to topics
//! (`rt_core::topics`) and calls or serves services (`rt_core::services`). Code written against a
//! node runs unchanged on the robot or on another machine; only its [`Link`] differs.
//!
//! | | topics | services |
//! |---|---|---|
//! | [`Link::Local`] (on the robot's host) | iceoryx2 shared memory (`rt-ipc`), zero-copy | Zenoh to robotd on this host |
//! | [`Link::Remote`] (another machine) | Zenoh (`rt-net`); robotd bridges them to local topics | Zenoh to robotd at the endpoint |
//!
//! The network is opened on first use, so a local node that only uses topics (the RT thread) never
//! touches it. A node that publishes, subscribes or serves over the network announces itself, and
//! shows up in `robotctl status` until it exits or its connection drops.
//!
//! A node and its ports are `!Send` (iceoryx2's are): each thread creates its own, during setup.

mod port;

use std::cell::OnceCell;

use rt_core::Message;
use rt_core::services::Service;
use rt_core::topics::Topic;
use rt_ipc::{IpcError, IpcNode};
use rt_net::{LivelinessToken, NetConfig, Session};
use serde::Serialize;
use serde::de::DeserializeOwned;

pub use port::{Publisher, Subscriber};
pub use rt_net::{CALL_TIMEOUT, DEFAULT_ENDPOINT, NetError, NodeEvent, NodeWatch, Server};

/// How a node reaches the robot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Link {
    /// On the robot's host: topics in shared memory, services via robotd's default endpoint.
    Local,
    /// From another machine (or process boundary), through robotd at this Zenoh endpoint, e.g.
    /// `tcp/10.0.0.5:7447`.
    Remote(String),
}

#[derive(Debug, thiserror::Error)]
pub enum NodeError {
    #[error(transparent)]
    Ipc(#[from] IpcError),
    #[error(transparent)]
    Net(#[from] NetError),
}

pub struct Node {
    robot: String,
    name: String,
    link: Link,
    listen: Vec<String>,
    ipc: Option<IpcNode>,
    net: OnceCell<Session>,
    presence: OnceCell<LivelinessToken>,
}

impl Node {
    /// `robot` scopes every topic and service; `name` identifies this node (in `robotctl status`
    /// and iceoryx2's tooling).
    pub fn new(robot: &str, name: &str, link: Link) -> Result<Self, NodeError> {
        let ipc = match link {
            Link::Local => Some(IpcNode::new(robot, name)?),
            Link::Remote(_) => None,
        };
        Ok(Self {
            robot: robot.to_string(),
            name: name.to_string(),
            link,
            listen: Vec::new(),
            ipc,
            net: OnceCell::new(),
            presence: OnceCell::new(),
        })
    }

    /// Accept network connections on `endpoints` instead of connecting out. For robotd.
    pub fn listen(mut self, endpoints: &[String]) -> Self {
        self.listen = endpoints.to_vec();
        self
    }

    pub fn robot(&self) -> &str {
        &self.robot
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn link(&self) -> &Link {
        &self.link
    }

    /// Publish `topic`. Each topic has one publisher on the robot: this fails if a live local
    /// process already publishes it. (A remote publisher is admitted by robotd's bridge, which
    /// refuses it while a local one holds the topic.)
    pub fn publisher<T: Message>(&self, topic: &Topic<T>) -> Result<Publisher<T>, NodeError> {
        Ok(match &self.ipc {
            Some(ipc) => Publisher::Ipc(ipc.publisher(topic)?),
            None => self.net_publisher(topic)?,
        })
    }

    pub fn subscriber<T: Message>(&self, topic: &Topic<T>) -> Result<Subscriber<T>, NodeError> {
        Ok(match &self.ipc {
            Some(ipc) => Subscriber::Ipc(ipc.subscriber(topic)?),
            None => self.net_subscriber(topic)?,
        })
    }

    /// Publish `topic` over the network whatever the link. For bridges.
    pub fn net_publisher<T: Message>(&self, topic: &Topic<T>) -> Result<Publisher<T>, NodeError> {
        let publisher = rt_net::NetPublisher::new(self.present()?, &self.robot, topic)?;
        Ok(Publisher::Net(publisher))
    }

    /// Subscribe to `topic` over the network whatever the link. For bridges.
    pub fn net_subscriber<T: Message>(&self, topic: &Topic<T>) -> Result<Subscriber<T>, NodeError> {
        let subscriber = rt_net::NetSubscriber::new(self.present()?, &self.robot, topic)?;
        Ok(Subscriber::Net(subscriber))
    }

    /// Call `service` on whichever node serves it, and wait (up to `rt_net::CALL_TIMEOUT`) for the
    /// response.
    pub fn call<Req, Resp>(
        &self,
        service: &Service<Req, Resp>,
        request: &Req,
    ) -> Result<Resp, NodeError>
    where
        Req: Serialize + DeserializeOwned,
        Resp: Serialize + DeserializeOwned,
    {
        Ok(rt_net::call(self.net()?, &self.robot, service, request)?)
    }

    /// A server for services this node provides: register handlers with `handle`, then `poll` it
    /// from this node's thread.
    pub fn server<Ctx>(&self) -> Result<Server<Ctx>, NodeError> {
        Ok(Server::new(self.present()?, &self.robot))
    }

    /// Watch which nodes are on the network.
    pub fn watch_nodes(&self) -> Result<NodeWatch, NodeError> {
        Ok(NodeWatch::new(self.net()?, &self.robot)?)
    }

    /// The network session, opened on first use.
    fn net(&self) -> Result<&Session, NodeError> {
        if let Some(session) = self.net.get() {
            return Ok(session);
        }
        let connect = match &self.link {
            _ if !self.listen.is_empty() => Vec::new(),
            Link::Remote(endpoint) => vec![endpoint.clone()],
            Link::Local => vec![DEFAULT_ENDPOINT.to_string()],
        };
        let session = rt_net::open(&NetConfig {
            listen: self.listen.clone(),
            connect,
        })?;
        Ok(self.net.get_or_init(|| session))
    }

    /// The network session, with this node announced on it. Taking part in topics or services
    /// announces a node; only calling services doesn't (robotctl isn't a participant).
    fn present(&self) -> Result<&Session, NodeError> {
        let session = self.net()?;
        if self.presence.get().is_none() {
            let token = rt_net::announce(session, &self.robot, &self.name)?;
            let _ = self.presence.set(token);
        }
        Ok(session)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytemuck::Zeroable;
    use rt_core::msgs::ArmState;
    use rt_core::services::{ENABLE, Reply};
    use rt_core::topics::STATE;
    use std::time::Duration;

    fn robot(test: &str) -> String {
        format!("test_{}_{test}", std::process::id())
    }

    fn state(cycle: u64) -> ArmState {
        ArmState {
            cycle,
            ..ArmState::zeroed()
        }
    }

    fn retry<T>(mut f: impl FnMut() -> Option<T>) -> Option<T> {
        (0..100).find_map(|_| {
            std::thread::sleep(Duration::from_millis(20));
            f()
        })
    }

    #[test]
    fn local_nodes_share_topics() {
        let robot = robot("local");
        let a = Node::new(&robot, "a", Link::Local).unwrap();
        let b = Node::new(&robot, "b", Link::Local).unwrap();
        let publisher = a.publisher(&STATE).unwrap();
        let subscriber = b.subscriber(&STATE).unwrap();
        assert!(matches!(publisher, Publisher::Ipc(_)));
        publisher.publish(&state(3)).unwrap();
        assert_eq!(subscriber.latest().unwrap().map(|s| s.cycle), Some(3));
        assert!(a.net.get().is_none(), "topics alone never open the network");
    }

    /// The same calls on remote nodes go over Zenoh; serving announces the node.
    #[test]
    fn remote_nodes_publish_and_call() {
        let robot = robot("remote");
        let endpoint = format!("tcp/127.0.0.1:{}", 18_000 + std::process::id() % 1000);
        let robotd = Node::new(&robot, "robotd", Link::Remote(endpoint.clone()))
            .unwrap()
            .listen(std::slice::from_ref(&endpoint));
        let policy = Node::new(&robot, "policy", Link::Remote(endpoint)).unwrap();

        let mut server = robotd.server::<()>().unwrap();
        server
            .handle(&ENABLE, |_, ()| Reply {
                ok: true,
                ..Default::default()
            })
            .unwrap();
        let states = policy.subscriber(&STATE).unwrap();
        let publisher = robotd.publisher(&STATE).unwrap();
        assert!(matches!(publisher, Publisher::Net(_)));
        let got = retry(|| {
            publisher.publish(&state(7)).unwrap();
            states.latest().unwrap()
        });
        assert_eq!(got.map(|s| s.cycle), Some(7));

        let watch = robotd.watch_nodes().unwrap();
        let mut seen = Vec::new();
        retry(|| {
            seen.extend(watch.drain().map(|e| e.label));
            seen.iter().any(|l| l.starts_with("policy/")).then_some(())
        })
        .expect("policy announced itself");

        let serving = std::thread::spawn(move || {
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            while server.poll(&mut (), Duration::from_millis(10)) == 0
                && std::time::Instant::now() < deadline
            {}
        });
        assert!(retry(|| policy.call(&ENABLE, &()).ok()).unwrap().ok);
        serving.join().unwrap();
    }
}
