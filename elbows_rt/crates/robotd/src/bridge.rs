//! Generic topic bridge between local iceoryx2 topics and Zenoh. Knows no message types: a route is
//! a declared `Topic<T>` plus a direction. Which topics are bridged comes from the setup file
//! (`[robotd.net] out`, `in`); any topic in `rt_core::topics` can be named there.
//!
//! - **Out** (local → Zenoh): "latest" topics forward the newest value each step; queues forward
//!   every message.
//! - **In** (Zenoh → local): local topics have a single publisher, so an inbound route claims it only
//!   while remote messages arrive and releases it after [`INBOUND_IDLE`], letting a local publisher
//!   take over. While a local publisher holds it, remote messages are refused (and logged once).

use std::rc::Rc;
use std::time::{Duration, Instant};

use rt_core::Message;
use rt_core::topics::{Pattern, Topic, TopicVisitor, visit_all};
use rt_node::{Node, Publisher, Subscriber};

pub const INBOUND_IDLE: Duration = Duration::from_secs(1);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Out,
    In,
}

trait Route {
    fn step(&mut self);
}

struct Outbound<T: Message> {
    pattern: Pattern,
    local: Subscriber<T>,
    remote: Publisher<T>,
}

impl<T: Message> Route for Outbound<T> {
    fn step(&mut self) {
        match self.pattern {
            Pattern::Latest => {
                if let Ok(Some(v)) = self.local.latest() {
                    let _ = self.remote.publish(&v);
                }
            }
            Pattern::Queue { .. } => {
                let _ = self.local.drain(|v| {
                    let _ = self.remote.publish(&v);
                });
            }
        }
    }
}

struct Inbound<T: Message> {
    topic: &'static Topic<T>,
    name: String,
    node: Rc<Node>,
    remote: Subscriber<T>,
    local: Option<Publisher<T>>,
    last_remote: Instant,
    refusing: bool,
}

impl<T: Message> Route for Inbound<T> {
    fn step(&mut self) {
        // Messages of the wrong type or layout version are dropped by the subscriber: they never
        // reach a local one.
        let mut received = Vec::new();
        let _ = self.remote.drain(|msg| received.push(msg));
        for msg in received {
            self.last_remote = Instant::now();
            if self.local.is_none() {
                match self.node.publisher(self.topic) {
                    Ok(p) => {
                        eprintln!("robotd: bridge: remote publisher now drives {}", self.name);
                        self.local = Some(p);
                        self.refusing = false;
                    }
                    Err(_) => {
                        if !self.refusing {
                            eprintln!(
                                "robotd: bridge: remote {} refused: a local publisher holds it",
                                self.name
                            );
                            self.refusing = true;
                        }
                        continue;
                    }
                }
            }
            if let Some(p) = &self.local {
                let _ = p.publish(&msg);
            }
        }
        if self.local.is_some() && self.last_remote.elapsed() > INBOUND_IDLE {
            eprintln!("robotd: bridge: remote {} idle; released", self.name);
            self.local = None;
        }
    }
}

pub struct Bridge {
    node: Rc<Node>,
    routes: Vec<Box<dyn Route>>,
}

impl Bridge {
    /// Bridge through `node`, a local node that also has the network.
    pub fn new(node: Rc<Node>) -> Self {
        Self {
            node,
            routes: Vec::new(),
        }
    }

    /// Bridge the topics named in `out` and `inbound` (by their declared names).
    pub fn from_names(node: Rc<Node>, out: &[String], inbound: &[String]) -> Result<Self, String> {
        struct Select<'a> {
            bridge: Bridge,
            out: &'a [String],
            inbound: &'a [String],
            declared: Vec<&'static str>,
            error: Option<String>,
        }
        impl TopicVisitor for Select<'_> {
            fn visit<T: Message>(&mut self, topic: &'static Topic<T>) {
                self.declared.push(topic.name);
                let wanted = |names: &[String]| names.iter().any(|n| n == topic.name);
                for (names, dir) in [(self.out, Direction::Out), (self.inbound, Direction::In)] {
                    if wanted(names)
                        && let Err(e) = self.bridge.route(topic, dir)
                    {
                        self.error.get_or_insert(e);
                    }
                }
            }
        }
        if let Some(both) = out.iter().find(|n| inbound.contains(n)) {
            return Err(format!("topic {both:?} can't be bridged both in and out"));
        }
        let mut select = Select {
            bridge: Bridge::new(node),
            out,
            inbound,
            declared: Vec::new(),
            error: None,
        };
        visit_all(&mut select);
        if let Some(e) = select.error {
            return Err(e);
        }
        if let Some(unknown) = out
            .iter()
            .chain(inbound)
            .find(|n| !select.declared.contains(&n.as_str()))
        {
            return Err(format!(
                "no topic {unknown:?} to bridge; declared: {:?}",
                select.declared
            ));
        }
        Ok(select.bridge)
    }

    pub fn route<T: Message>(
        &mut self,
        topic: &'static Topic<T>,
        dir: Direction,
    ) -> Result<(), String> {
        let name = format!("{}/{}", self.node.robot(), topic.name);
        let route: Box<dyn Route> = match dir {
            Direction::Out => Box::new(Outbound {
                pattern: topic.pattern,
                local: self.node.subscriber(topic).map_err(|e| e.to_string())?,
                remote: self.node.net_publisher(topic).map_err(|e| e.to_string())?,
            }),
            Direction::In => Box::new(Inbound {
                topic,
                name,
                node: self.node.clone(),
                remote: self.node.net_subscriber(topic).map_err(|e| e.to_string())?,
                local: None,
                last_remote: Instant::now(),
                refusing: false,
            }),
        };
        self.routes.push(route);
        Ok(())
    }

    pub fn step(&mut self) {
        for route in &mut self.routes {
            route.step();
        }
    }
}
