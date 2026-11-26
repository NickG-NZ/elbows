//! Request/response services, declared once like topics: the name, request type and response type
//! travel together. Requests and replies are JSON (readable; easy from Python and the shell), unlike
//! topics, which are raw bytes.
//!
//! A server registers a handler per service; a client calls one by its declaration. The transport
//! (Zenoh, in `rt-net`) knows nothing about any particular service.

use std::marker::PhantomData;

use serde::{Deserialize, Serialize};

#[derive(Debug)]
pub struct Service<Req, Resp> {
    pub name: &'static str,
    _types: PhantomData<fn(Req) -> Resp>,
}

impl<Req, Resp> Service<Req, Resp> {
    pub const fn new(name: &'static str) -> Self {
        Self {
            name,
            _types: PhantomData,
        }
    }
}

// ---- robotd's services ---------------------------------------------------------------------------

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
    /// Nodes on the network, as `<name>/<id>` (local nodes that only use topics don't appear).
    pub nodes: Vec<String>,
}

pub const STATUS: Service<(), Reply> = Service::new("status");
/// Disabled → Hold at the measured pose.
pub const ENABLE: Service<(), Reply> = Service::new("enable");
/// Drop the plan and hold.
pub const HOLD: Service<(), Reply> = Service::new("hold");
/// → Disabled.
pub const STOP: Service<(), Reply> = Service::new("stop");

/// robotd's operator commands, for CLIs that pick one by name.
pub const COMMANDS: [&Service<(), Reply>; 4] = [&STATUS, &ENABLE, &HOLD, &STOP];
