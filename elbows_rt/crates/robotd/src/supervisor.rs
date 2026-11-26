//! The supervisor: robotd's non-RT control side and its only external interface. One thread. Its
//! loop waits briefly for service requests, then handles status, nodes, the bridge and health.
//!
//! - **Services** (`rt_core::services`) are served by handlers registered below, over Zenoh. Mode
//!   commands go to the RT thread in-process; it decides, and its verdict is the reply.
//! - **Nodes**: which nodes are on the network (`rt_node`), for `robotctl status`.
//! - **Bridge**: whichever topics the setup file names (see [`Bridge`]).
//!
//! The RT thread never touches Zenoh: the supervisor requests, the RT thread decides.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, TrySendError};
use std::time::{Duration, Instant};

use rt_core::msgs::{Mode, Status};
use rt_core::services::{self, Reply, StatusReport};
use std::rc::Rc;

use rt_node::Node;
use rt_node::{NodeWatch, Server};

use crate::bridge::Bridge;
use crate::controller::ModeCommand;
use crate::rt_loop::{CommandOutcome, CommandRequest};

const POLL: Duration = Duration::from_millis(1);
const HEALTH_PERIOD: Duration = Duration::from_millis(100);
const REPORT_PERIOD: Duration = Duration::from_secs(1);
const STALL_AFTER: Duration = Duration::from_millis(500);
/// How long to wait for the RT thread's verdict on a command.
const COMMAND_DEADLINE: Duration = Duration::from_millis(250);

/// The supervisor's ends of its in-process links to the RT thread.
pub struct RtLink {
    pub commands: SyncSender<CommandRequest>,
    pub outcomes: Receiver<CommandOutcome>,
    pub status: Receiver<Status>,
    pub heartbeat: Arc<AtomicU64>,
}

/// What service handlers act on.
pub struct Core {
    rt: RtLink,
    status: Option<Status>,
    nodes: BTreeSet<String>,
    next_command_id: u64,
    stalled: bool,
}

pub struct Supervisor {
    core: Core,
    server: Server<Core>,
    nodes: NodeWatch,
    bridge: Bridge,
    _node: Rc<Node>,
}

impl Supervisor {
    /// `node` is robotd's network-facing node (local, listening); `bridge` routes through it.
    pub fn new(node: Rc<Node>, rt: RtLink, bridge: Bridge) -> Result<Self, String> {
        let mut server = node.server().map_err(|e| e.to_string())?;
        let claim = |e: rt_node::NetError| format!("{e} (is another robotd running?)");
        server
            .handle(&services::STATUS, |c: &mut Core, ()| {
                c.reply(true, String::new())
            })
            .map_err(claim)?;
        for (service, cmd) in [
            (&services::ENABLE, ModeCommand::Enable),
            (&services::HOLD, ModeCommand::Hold),
            (&services::STOP, ModeCommand::Stop),
        ] {
            let name = service.name;
            server
                .handle(service, move |c: &mut Core, ()| c.forward(name, cmd))
                .map_err(claim)?;
        }
        let nodes = node.watch_nodes().map_err(|e| e.to_string())?;
        let core = Core {
            rt,
            status: None,
            nodes: BTreeSet::new(),
            next_command_id: 0,
            stalled: false,
        };
        Ok(Self {
            core,
            server,
            nodes,
            bridge,
            _node: node,
        })
    }

    /// Run until `stop` is set or `rt_done` reports the RT thread has exited.
    pub fn run(&mut self, stop: &AtomicBool, rt_done: impl Fn() -> bool) {
        let mut last_hb = (
            self.core.rt.heartbeat.load(Ordering::Acquire),
            Instant::now(),
        );
        let (mut last_health, mut last_report) = (Instant::now(), Instant::now());
        while !stop.load(Ordering::Relaxed) && !rt_done() {
            self.server.poll(&mut self.core, POLL); // the wait for requests paces the loop
            if let Some(status) = self.core.rt.status.try_iter().last() {
                self.core.status = Some(status);
            }
            self.track_nodes();
            self.bridge.step();

            if last_health.elapsed() >= HEALTH_PERIOD {
                last_health = Instant::now();
                let hb = self.core.rt.heartbeat.load(Ordering::Acquire);
                if hb != last_hb.0 {
                    if self.core.stalled {
                        eprintln!("robotd: RT thread running again at tick {hb}");
                    }
                    last_hb = (hb, Instant::now());
                    self.core.stalled = false;
                } else if !self.core.stalled && last_hb.1.elapsed() > STALL_AFTER {
                    eprintln!("robotd: RT thread stalled at tick {hb}; refusing commands");
                    self.core.stalled = true;
                }
            }
            if last_report.elapsed() >= REPORT_PERIOD {
                last_report = Instant::now();
                self.core.report();
            }
        }
    }

    fn track_nodes(&mut self) {
        for event in self.nodes.drain() {
            let changed = if event.alive {
                self.core.nodes.insert(event.label.clone())
            } else {
                self.core.nodes.remove(&event.label)
            };
            if changed {
                let what = if event.alive { "connected" } else { "gone" };
                eprintln!("robotd: node {} {what}", event.label);
            }
        }
    }
}

impl Core {
    /// Hand a mode command to the RT thread and wait for its verdict.
    fn forward(&mut self, name: &str, cmd: ModeCommand) -> Reply {
        if self.stalled {
            return self.reply(false, "RT thread stalled".into());
        }
        self.next_command_id += 1;
        let id = self.next_command_id;
        match self.rt.commands.try_send(CommandRequest { id, cmd }) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) => {
                return self.reply(false, "busy: too many pending commands".into());
            }
            Err(TrySendError::Disconnected(_)) => {
                return self.reply(false, "RT thread has exited".into());
            }
        }
        let deadline = Instant::now() + COMMAND_DEADLINE;
        while let Some(left) = deadline.checked_duration_since(Instant::now()) {
            match self.rt.outcomes.recv_timeout(left) {
                Ok(out) if out.id == id => {
                    eprintln!("robotd: {name} → {} ({:?})", out.mode.name(), out.result);
                    if let Some(s) = &mut self.status {
                        s.last.mode = out.mode as u32; // don't wait for the next status to show it
                    }
                    return match out.result {
                        Ok(()) => self.reply(true, format!("{name}: now {}", out.mode.name())),
                        Err(why) => self.reply(false, format!("{name} refused: {why}")),
                    };
                }
                Ok(_) => continue, // outcome of an earlier command that timed out
                Err(_) => break,
            }
        }
        self.reply(false, format!("{name}: no answer from the RT thread"))
    }

    fn reply(&self, ok: bool, message: String) -> Reply {
        Reply {
            ok,
            message,
            status: self.status_report(),
        }
    }

    fn status_report(&self) -> StatusReport {
        let nodes = self.nodes.iter().cloned().collect();
        let Some(s) = &self.status else {
            return StatusReport {
                mode: "starting".into(),
                nodes,
                ..Default::default()
            };
        };
        StatusReport {
            mode: Mode::from_u32(s.last.mode)
                .map_or("unknown", Mode::name)
                .into(),
            plan_seq: s.last.plan_seq,
            plans_rejected: s.last.plans_rejected,
            cycles: s.stats.cycles,
            missed: s.stats.missed,
            latency_p99_us: s.stats.latency.quantile_us(0.99),
            latency_max_us: s.stats.latency.max_ns / 1000,
            exec_p99_us: s.stats.exec.quantile_us(0.99),
            nodes,
        }
    }

    fn report(&self) {
        let r = self.status_report();
        eprintln!(
            "robotd: {:<8} plan {:<5} rejected {:<3} | cycles {} missed {} | latency p99 <{}µs max {}µs | exec p99 <{}µs | nodes {}",
            r.mode,
            r.plan_seq,
            r.plans_rejected,
            r.cycles,
            r.missed,
            r.latency_p99_us,
            r.latency_max_us,
            r.exec_p99_us,
            r.nodes.len(),
        );
    }
}
