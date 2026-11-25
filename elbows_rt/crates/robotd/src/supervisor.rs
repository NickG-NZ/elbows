//! The supervisor: robotd's non-RT control side and its only external interface. One thread:
//!
//! - **Commands** (`robotctl`, policies, remote tools): Zenoh queries on `elbows/<robot>/ctl/*`.
//!   `status` is answered here; mode commands go to the RT thread in-process, which decides, and its
//!   verdict is the reply.
//! - **Sessions**: Zenoh liveliness tokens under `elbows/<robot>/session/**`, so robotd knows who is
//!   connected and notices at once when a client goes away.
//! - **Bridge**: `state` and `status` out to Zenoh; `plan` in from a remote policy, republished on
//!   the local iceoryx2 topic (see [`Bridge`]).
//! - **Health**: RT heartbeat, status reporting.
//!
//! The RT thread never touches Zenoh: the supervisor requests, the RT thread decides.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, TrySendError};
use std::time::{Duration, Instant};

use rt_core::msgs::{ArmState, Mode, Plan, Status};
use rt_core::topics::{PLAN, STATE, STATUS};
use rt_net::{Command, NetPublisher, NetSubscriber, Reply, Session, StatusReport};
use zenoh::Wait;
use zenoh::handlers::FifoChannelHandler;
use zenoh::query::{Query, Queryable};
use zenoh::sample::{Sample, SampleKind};

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

pub struct Supervisor {
    rt: RtLink,
    queries: Queryable<FifoChannelHandler<Query>>,
    session_events: zenoh::pubsub::Subscriber<FifoChannelHandler<Sample>>,
    sessions: BTreeSet<String>,
    bridge: Bridge,
    status: Option<Status>,
    next_command_id: u64,
    stalled: bool,
    _net: Session,
}

impl Supervisor {
    pub fn new(robot: &str, rt: RtLink, net: Session) -> Result<Self, String> {
        let queries = net
            .declare_queryable(rt_net::ctl_wildcard(robot))
            .wait()
            .map_err(|e| format!("declare command queryable: {e}"))?;
        let session_events = net
            .liveliness()
            .declare_subscriber(rt_net::sessions_wildcard(robot))
            .history(true)
            .wait()
            .map_err(|e| format!("watch sessions: {e}"))?;
        let bridge = Bridge::new(robot, &net)?;
        Ok(Self {
            rt,
            queries,
            session_events,
            sessions: BTreeSet::new(),
            bridge,
            status: None,
            next_command_id: 0,
            stalled: false,
            _net: net,
        })
    }

    /// Run until `stop` is set or `rt_done` reports the RT thread has exited.
    pub fn run(&mut self, stop: &AtomicBool, rt_done: impl Fn() -> bool) {
        let mut last_hb = (self.rt.heartbeat.load(Ordering::Acquire), Instant::now());
        let (mut last_health, mut last_report) = (Instant::now(), Instant::now());
        while !stop.load(Ordering::Relaxed) && !rt_done() {
            // The wait for a command paces the loop.
            if let Ok(Some(query)) = self.queries.recv_timeout(POLL) {
                self.answer(&query);
            }
            for status in self.rt.status.try_iter() {
                self.bridge.publish_status(&status);
                self.status = Some(status);
            }
            self.track_sessions();
            self.bridge.step();

            if last_health.elapsed() >= HEALTH_PERIOD {
                last_health = Instant::now();
                let hb = self.rt.heartbeat.load(Ordering::Acquire);
                if hb != last_hb.0 {
                    if self.stalled {
                        eprintln!("robotd: RT thread running again at tick {hb}");
                    }
                    last_hb = (hb, Instant::now());
                    self.stalled = false;
                } else if !self.stalled && last_hb.1.elapsed() > STALL_AFTER {
                    eprintln!("robotd: RT thread stalled at tick {hb}; refusing commands");
                    self.stalled = true;
                }
            }
            if last_report.elapsed() >= REPORT_PERIOD {
                last_report = Instant::now();
                self.report();
            }
        }
    }

    fn answer(&mut self, query: &Query) {
        let reply = match Command::from_key(query.key_expr().as_str()) {
            None => {
                let _ = query
                    .reply_err(format!("unknown command {}", query.key_expr()))
                    .wait();
                return;
            }
            Some(Command::Status) => self.reply(true, String::new()),
            Some(cmd) => self.forward(cmd),
        };
        let json = serde_json::to_vec(&reply).expect("reply serializes");
        if let Err(e) = query.reply(query.key_expr().clone(), json).wait() {
            eprintln!("robotd: replying to {}: {e}", query.key_expr());
        }
    }

    /// Hand a mode command to the RT thread and wait for its verdict.
    fn forward(&mut self, cmd: Command) -> Reply {
        let mode_cmd = match cmd {
            Command::Enable => ModeCommand::Enable,
            Command::Hold => ModeCommand::Hold,
            Command::Stop => ModeCommand::Stop,
            Command::Status => unreachable!("answered by the supervisor"),
        };
        if self.stalled {
            return self.reply(false, "RT thread stalled".into());
        }
        self.next_command_id += 1;
        let id = self.next_command_id;
        match self
            .rt
            .commands
            .try_send(CommandRequest { id, cmd: mode_cmd })
        {
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
                    eprintln!(
                        "robotd: {} → {} ({:?})",
                        cmd.name(),
                        out.mode.name(),
                        out.result
                    );
                    let message = match out.result {
                        Ok(()) => format!("{}: now {}", cmd.name(), out.mode.name()),
                        Err(why) => format!("{} refused: {why}", cmd.name()),
                    };
                    if let Some(s) = &mut self.status {
                        s.last.mode = out.mode as u32; // don't wait for the next status to show it
                    }
                    return self.reply(out.result.is_ok(), message);
                }
                Ok(_) => continue, // outcome of an earlier command that timed out
                Err(_) => break,
            }
        }
        self.reply(
            false,
            format!("{}: no answer from the RT thread", cmd.name()),
        )
    }

    fn reply(&self, ok: bool, message: String) -> Reply {
        Reply {
            ok,
            message,
            status: self.status_report(),
        }
    }

    fn status_report(&self) -> StatusReport {
        let sessions = self.sessions.iter().cloned().collect();
        let Some(s) = &self.status else {
            return StatusReport {
                mode: "starting".into(),
                sessions,
                ..Default::default()
            };
        };
        let mode = Mode::from_u32(s.last.mode).map_or("unknown", Mode::name);
        StatusReport {
            mode: mode.into(),
            plan_seq: s.last.plan_seq,
            plans_rejected: s.last.plans_rejected,
            cycles: s.stats.cycles,
            missed: s.stats.missed,
            latency_p99_us: s.stats.latency.quantile_us(0.99),
            latency_max_us: s.stats.latency.max_ns / 1000,
            exec_p99_us: s.stats.exec.quantile_us(0.99),
            sessions,
        }
    }

    fn track_sessions(&mut self) {
        while let Ok(Some(sample)) = self.session_events.try_recv() {
            let name = rt_net::session_name(sample.key_expr().as_str()).to_string();
            match sample.kind() {
                SampleKind::Put => {
                    if self.sessions.insert(name.clone()) {
                        eprintln!("robotd: session {name} connected");
                    }
                }
                SampleKind::Delete => {
                    if self.sessions.remove(&name) {
                        eprintln!("robotd: session {name} gone");
                    }
                }
            }
        }
    }

    fn report(&self) {
        let r = self.status_report();
        eprintln!(
            "robotd: {:<8} plan {:<5} rejected {:<3} | cycles {} missed {} | latency p99 <{}µs max {}µs | exec p99 <{}µs | sessions {}",
            r.mode,
            r.plan_seq,
            r.plans_rejected,
            r.cycles,
            r.missed,
            r.latency_p99_us,
            r.latency_max_us,
            r.exec_p99_us,
            r.sessions.len(),
        );
    }
}

/// Carries topics between the local iceoryx2 topics and Zenoh.
///
/// - Out: `state` (latest, at up to the supervisor's poll rate) and `status`.
/// - In: `plan` from a remote policy. The bridge takes the local single-publisher `plan` topic only
///   while remote plans arrive, and releases it after [`REMOTE_PLAN_IDLE`], so a local policy can
///   take over. If a local policy holds it, remote plans are refused.
pub struct Bridge {
    robot: String,
    node: rt_ipc::Node,
    state_in: rt_ipc::Subscriber<ArmState>,
    state_out: NetPublisher<ArmState>,
    status_out: NetPublisher<Status>,
    plan_in: NetSubscriber<Plan>,
    plan_out: Option<rt_ipc::Publisher<Plan>>,
    last_remote_plan: Instant,
    refusing: bool,
}

pub const REMOTE_PLAN_IDLE: Duration = Duration::from_secs(1);

impl Bridge {
    fn new(robot: &str, net: &Session) -> Result<Self, String> {
        let node = rt_ipc::Node::new(robot, "robotd-bridge").map_err(|e| e.to_string())?;
        Ok(Self {
            robot: robot.into(),
            state_in: node.subscriber(&STATE).map_err(|e| e.to_string())?,
            state_out: NetPublisher::new(net, robot, &STATE).map_err(|e| e.to_string())?,
            status_out: NetPublisher::new(net, robot, &STATUS).map_err(|e| e.to_string())?,
            plan_in: NetSubscriber::new(net, robot, &PLAN).map_err(|e| e.to_string())?,
            plan_out: None,
            last_remote_plan: Instant::now(),
            refusing: false,
            node,
        })
    }

    fn step(&mut self) {
        if let Ok(Some(state)) = self.state_in.latest() {
            let _ = self.state_out.publish(&state);
        }
        for plan in self.plan_in.drain() {
            let Ok(plan) = plan else {
                continue; // wrong type or size: never reaches the RT thread
            };
            self.last_remote_plan = Instant::now();
            if self.plan_out.is_none() {
                match self.node.publisher(&PLAN) {
                    Ok(p) => {
                        eprintln!("robotd: remote policy publishing {}/plan", self.robot);
                        self.plan_out = Some(p);
                        self.refusing = false;
                    }
                    Err(_) => {
                        if !self.refusing {
                            eprintln!(
                                "robotd: remote plans refused: a local policy holds {}/plan",
                                self.robot
                            );
                            self.refusing = true;
                        }
                        continue;
                    }
                }
            }
            if let Some(p) = &self.plan_out {
                let _ = p.publish(&plan);
            }
        }
        if self.plan_out.is_some() && self.last_remote_plan.elapsed() > REMOTE_PLAN_IDLE {
            eprintln!("robotd: remote policy idle; released {}/plan", self.robot);
            self.plan_out = None;
        }
    }

    fn publish_status(&self, status: &Status) {
        let _ = self.status_out.publish(status);
    }
}
