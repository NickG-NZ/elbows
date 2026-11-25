//! Typed topics over iceoryx2. Drivers, the controller and policies use this API, never iceoryx2
//! directly, so the transport can be swapped.
//!
//! Topics are declared in `rt_core::topics` (name, message type and pattern together). A [`Node`]
//! prefixes every topic with a namespace, normally the robot name: `so101/plan`.
//!
//! - Every topic has **one publisher** and up to [`MAX_SUBSCRIBERS`] subscribers, each with its own
//!   buffer. A full buffer drops its oldest sample, so a stalled subscriber never blocks the
//!   publisher.
//! - [`Pattern::Latest`]: buffer of 1, and a late subscriber gets the last value on connecting.
//! - [`Pattern::Queue`]: buffer of `depth`, every sample in order (until it overflows).
//!
//! Ports use iceoryx2's single-threaded `ipc` variant: no locks inside `publish`/`receive`, but a
//! [`Node`] and its ports are `!Send`. Each thread creates its own, during setup.
//!
//! In the RT loop, `publish` and `latest`/`drain` don't allocate. They may make syscalls when a
//! peer connects or disconnects, which iceoryx2 picks up on the next call.

use std::fmt::Debug;
use std::marker::PhantomData;
use std::sync::Once;

use bytemuck::Pod;
use iceoryx2::port::publisher::Publisher as IoxPublisher;
use iceoryx2::port::subscriber::Subscriber as IoxSubscriber;
use iceoryx2::prelude::*;
use iceoryx2::service::port_factory::publish_subscribe::PortFactory;
use rt_core::topics::{Pattern, Topic};

/// Subscribers per topic (policy, logger, viewer, bridges, …).
pub const MAX_SUBSCRIBERS: usize = 8;
/// Processes that may have a topic open at once.
pub const MAX_NODES: usize = 16;

/// A type that can be sent through a topic: plain data, valid for any bytes a peer writes
/// (`Pod`), with a fixed cross-process layout and name (`ZeroCopySend`).
pub trait Message: Pod + ZeroCopySend + Debug + Send + Sync + 'static {}
impl<T: Pod + ZeroCopySend + Debug + Send + Sync + 'static> Message for T {}

#[derive(Debug, thiserror::Error)]
pub enum IpcError {
    /// Creating a node, service or port failed. Only happens during setup.
    #[error("{what}: {detail}")]
    Setup { what: String, detail: String },
    /// Sending failed. Allocation-free, so it can be returned from the RT loop.
    #[error("send failed")]
    Send,
    /// Receiving failed. Allocation-free, so it can be returned from the RT loop.
    #[error("receive failed")]
    Receive,
}

fn setup<E: Debug>(what: impl Into<String>) -> impl FnOnce(E) -> IpcError {
    let what = what.into();
    move |e| IpcError::Setup {
        what,
        detail: format!("{e:?}"),
    }
}

/// One participant (a process, or a thread within one). Owns its ports' resources.
pub struct Node {
    node: iceoryx2::node::Node<ipc::Service>,
    namespace: String,
}

impl Node {
    /// `namespace` prefixes every topic (normally the robot name); `name` identifies this node in
    /// iceoryx2's tooling (`iox2 node list`).
    pub fn new(namespace: &str, name: &str) -> Result<Self, IpcError> {
        // iceoryx2 logs routine events (dead-node cleanup, default config) as warnings with large
        // dumps. Errors only, unless IOX2_LOG_LEVEL says otherwise.
        static QUIET: Once = Once::new();
        QUIET.call_once(|| set_log_level_from_env_or(LogLevel::Error));
        let node_name = NodeName::new(name).map_err(setup(format!("node name {name:?}")))?;
        let node = NodeBuilder::new()
            .name(&node_name)
            // The application owns SIGINT/SIGTERM handling.
            .signal_handling_mode(SignalHandlingMode::Disabled)
            .create::<ipc::Service>()
            .map_err(setup("create iceoryx2 node"))?;
        Ok(Self {
            node,
            namespace: namespace.to_string(),
        })
    }

    pub fn topic_name<T>(&self, topic: &Topic<T>) -> String {
        format!("{}/{}", self.namespace, topic.name)
    }

    fn service<T: Message>(
        &self,
        topic: &Topic<T>,
    ) -> Result<PortFactory<ipc::Service, T, ()>, IpcError> {
        let name = self.topic_name(topic);
        let service_name =
            ServiceName::new(&name).map_err(setup(format!("topic name {name:?}")))?;
        let (depth, history) = match topic.pattern {
            Pattern::Latest => (1, 1),
            Pattern::Queue { depth } => (depth, 0),
        };
        // Creator and opener pass the same settings (they come from one `Topic` constant), so a
        // mismatch in type or settings means a stale or foreign build and fails here.
        self.node
            .service_builder(&service_name)
            .publish_subscribe::<T>()
            .enable_safe_overflow(true)
            .subscriber_max_buffer_size(depth)
            .history_size(history)
            .max_publishers(1)
            .max_subscribers(MAX_SUBSCRIBERS)
            .max_nodes(MAX_NODES)
            .open_or_create()
            .map_err(setup(format!("open topic {name}")))
    }

    /// The topic's single publisher. Fails if another live process already publishes it.
    pub fn publisher<T: Message>(&self, topic: &Topic<T>) -> Result<Publisher<T>, IpcError> {
        let port = self
            .service(topic)?
            .publisher_builder()
            .backpressure_strategy(BackpressureStrategy::DiscardData)
            .create()
            .map_err(setup(format!("publish {}", self.topic_name(topic))))?;
        Ok(Publisher { port })
    }

    pub fn subscriber<T: Message>(&self, topic: &Topic<T>) -> Result<Subscriber<T>, IpcError> {
        let port = self
            .service(topic)?
            .subscriber_builder()
            .create()
            .map_err(setup(format!("subscribe {}", self.topic_name(topic))))?;
        Ok(Subscriber {
            port,
            _msg: PhantomData,
        })
    }
}

pub struct Publisher<T: Message> {
    port: IoxPublisher<ipc::Service, T, ()>,
}

impl<T: Message> Publisher<T> {
    /// Send a copy of `value` to every subscriber. Never blocks.
    pub fn publish(&self, value: &T) -> Result<(), IpcError> {
        self.port
            .send_copy(*value)
            .map(|_| ())
            .map_err(|_| IpcError::Send)
    }
}

pub struct Subscriber<T: Message> {
    port: IoxSubscriber<ipc::Service, T, ()>,
    _msg: PhantomData<T>,
}

impl<T: Message> Subscriber<T> {
    /// The newest unread sample, if any; older unread ones are discarded. Copied out of shared
    /// memory: if it came from another process, validate it before use.
    pub fn latest(&self) -> Result<Option<T>, IpcError> {
        let mut last = None;
        while let Some(sample) = self.port.receive().map_err(|_| IpcError::Receive)? {
            last = Some(*sample);
        }
        Ok(last)
    }

    /// Every unread sample, oldest first. Returns how many there were.
    pub fn drain(&self, mut f: impl FnMut(T)) -> Result<usize, IpcError> {
        let mut n = 0;
        while let Some(sample) = self.port.receive().map_err(|_| IpcError::Receive)? {
            f(*sample);
            n += 1;
        }
        Ok(n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use iceoryx2::prelude::ZeroCopySend;

    #[repr(C)]
    #[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable, ZeroCopySend)]
    #[type_name("elbows.test.Counter")]
    struct Counter {
        n: u64,
    }

    const LATEST: Topic<Counter> = Topic::latest("counter");
    const QUEUE: Topic<Counter> = Topic::queue("counter_queue", 4);

    /// Each test uses its own namespace, so parallel tests (and other processes) don't collide.
    fn ns(test: &str) -> String {
        format!("test_{}_{test}", std::process::id())
    }

    #[test]
    fn latest_gives_newest_to_every_subscriber() {
        let ns = ns("latest");
        let node = Node::new(&ns, "test").unwrap();
        let publisher = node.publisher(&LATEST).unwrap();
        let a = node.subscriber(&LATEST).unwrap();
        let b = node.subscriber(&LATEST).unwrap();
        assert_eq!(a.latest().unwrap(), None);
        for n in 1..=3 {
            publisher.publish(&Counter { n }).unwrap();
        }
        assert_eq!(a.latest().unwrap(), Some(Counter { n: 3 }));
        assert_eq!(a.latest().unwrap(), None);
        assert_eq!(
            b.latest().unwrap(),
            Some(Counter { n: 3 }),
            "fan-out: b has its own buffer"
        );
    }

    #[test]
    fn late_subscriber_gets_last_value() {
        let ns = ns("history");
        let node = Node::new(&ns, "test").unwrap();
        let publisher = node.publisher(&LATEST).unwrap();
        publisher.publish(&Counter { n: 7 }).unwrap();
        let late = node.subscriber(&LATEST).unwrap();
        publisher.publish(&Counter { n: 8 }).unwrap(); // a send also delivers history to new subscribers
        assert_eq!(late.latest().unwrap(), Some(Counter { n: 8 }));
    }

    #[test]
    fn queue_keeps_order_and_drops_oldest_when_full() {
        let ns = ns("queue");
        let node = Node::new(&ns, "test").unwrap();
        let publisher = node.publisher(&QUEUE).unwrap();
        let sub = node.subscriber(&QUEUE).unwrap();
        for n in 1..=6 {
            publisher.publish(&Counter { n }).unwrap();
        }
        let mut got = Vec::new();
        sub.drain(|c| got.push(c.n)).unwrap();
        assert_eq!(got, [3, 4, 5, 6]);
    }

    #[test]
    fn only_one_publisher_per_topic() {
        let ns = ns("single");
        let node = Node::new(&ns, "test").unwrap();
        let _first = node.publisher(&LATEST).unwrap();
        assert!(matches!(
            node.publisher(&LATEST),
            Err(IpcError::Setup { .. })
        ));
    }
}
