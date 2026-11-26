//! The RT thread's cycle. Everything it touches is created before `run` starts, on the RT thread
//! itself (iceoryx2 ports can't move between threads).
//!
//! Talks to the supervisor in-process only, never through the network or topics: mode commands in,
//! outcomes and status out, over bounded channels allocated up front (`try_recv`/`try_send` never
//! block or allocate).

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, SyncSender};

use rt_core::msgs::{ArmState, Mode, Plan, Status};
use rt_core::stats::CycleStats;
use rt_core::time::monotonic_ns;
use rt_core::topics::{PLAN, STATE, STATUS};
use rt_core::trigger::Trigger;
use rt_devices::CyclicDevice;
use rt_node::{Link, Node, NodeError, Publisher, Subscriber};

use crate::controller::{Controller, ModeCommand};

const STATUS_PERIOD_NS: u64 = 100_000_000;
/// Commands applied per cycle at most, so a flood can't stretch a cycle.
const MAX_COMMANDS_PER_CYCLE: usize = 4;

pub struct CommandRequest {
    pub id: u64,
    pub cmd: ModeCommand,
}

pub struct CommandOutcome {
    pub id: u64,
    pub result: Result<(), &'static str>,
    pub mode: Mode,
}

/// The RT thread's ends of its in-process links to the supervisor.
pub struct SupervisorLink {
    pub commands: Receiver<CommandRequest>,
    pub outcomes: SyncSender<CommandOutcome>,
    pub status: SyncSender<Status>,
    /// Last tick index; the supervisor checks that it advances.
    pub heartbeat: Arc<AtomicU64>,
}

pub struct RtLoop {
    trigger: Box<dyn Trigger>,
    devices: Vec<Box<dyn CyclicDevice>>,
    controller: Controller,
    plans: Subscriber<Plan>,
    state: Publisher<ArmState>,
    status: Publisher<Status>,
    link: SupervisorLink,
    max_consecutive_overruns: u32,
    _node: Node,
}

impl RtLoop {
    /// Create the RT thread's node and ports. Call on the RT thread, before `run`.
    pub fn new(
        robot: &str,
        trigger: Box<dyn Trigger>,
        devices: Vec<Box<dyn CyclicDevice>>,
        controller: Controller,
        link: SupervisorLink,
        max_consecutive_overruns: u32,
    ) -> Result<Self, NodeError> {
        // Local: topics in shared memory. The RT node never opens the network.
        let node = Node::new(robot, "robotd-rt", Link::Local)?;
        Ok(Self {
            plans: node.subscriber(&PLAN)?,
            state: node.publisher(&STATE)?,
            status: node.publisher(&STATUS)?,
            trigger,
            devices,
            controller,
            link,
            max_consecutive_overruns,
            _node: node,
        })
    }

    pub fn run(mut self, stop: &AtomicBool) {
        let mut stats = CycleStats::default();
        let mut consecutive_overruns = 0;
        let mut last_status_ns = 0;
        let mut q_prev = self.controller.step(0);
        while !stop.load(Ordering::Relaxed) {
            let tick = self.trigger.wait();
            let now = tick.now_ns;

            // 1. Read inputs.
            let mut device_failed = false;
            for d in &mut self.devices {
                device_failed |= d.read(&tick).is_err();
            }

            // 2. State estimation. No devices yet: measured = commanded (an ideal servo).
            let q_meas = q_prev;

            // Mode commands from the supervisor, applied at the cycle boundary. We decide.
            for req in self.link.commands.try_iter().take(MAX_COMMANDS_PER_CYCLE) {
                let result = self.controller.command(req.cmd, &q_meas);
                let mode = self.controller.mode();
                let _ = self.link.outcomes.try_send(CommandOutcome {
                    id: req.id,
                    result,
                    mode,
                });
            }

            // 3. Take the newest plan, if any, and sample the active one at `now`.
            match self.plans.latest() {
                Ok(Some(plan)) => {
                    let _ = self.controller.offer(&plan, now);
                }
                Ok(None) => {}
                Err(_) => self.controller.reject(),
            }
            // 4. Low-level control: position targets straight through for now.
            let q_cmd = self.controller.step(now);
            q_prev = q_cmd;

            // 5. Bus I/O.
            for d in &mut self.devices {
                device_failed |= d.write(&tick).is_err();
            }

            // Safety: latched faults.
            consecutive_overruns = if tick.missed > 0 {
                consecutive_overruns + 1
            } else {
                0
            };
            if device_failed || consecutive_overruns >= self.max_consecutive_overruns {
                self.controller.fault();
            }

            // 6. Publish.
            let state = ArmState {
                stamp_ns: now,
                cycle: tick.index,
                plan_seq: self.controller.plan_seq(),
                mode: self.controller.mode() as u32,
                plans_rejected: self.controller.rejected(),
                q_cmd,
                q_meas,
            };
            let _ = self.state.publish(&state);
            stats.record(tick.latency_ns(), monotonic_ns() - now, tick.missed);
            self.link.heartbeat.store(tick.index, Ordering::Release);
            if now - last_status_ns >= STATUS_PERIOD_NS {
                let status = Status { stats, last: state };
                let _ = self.status.publish(&status);
                let _ = self.link.status.try_send(status);
                last_status_ns = now;
            }
        }
    }
}
