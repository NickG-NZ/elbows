//! The RT thread's cycle. Everything it touches is created before `run` starts, on the RT thread
//! itself (iceoryx2 ports can't move between threads).

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use rt_core::msgs::{ArmState, Plan, Status};
use rt_core::stats::CycleStats;
use rt_core::time::monotonic_ns;
use rt_core::topics::{PLAN, STATE, STATUS};
use rt_core::trigger::Trigger;
use rt_devices::CyclicDevice;
use rt_ipc::{IpcError, Node, Publisher, Subscriber};

use crate::controller::Controller;

const STATUS_PERIOD_NS: u64 = 100_000_000;

pub struct RtLoop {
    trigger: Box<dyn Trigger>,
    devices: Vec<Box<dyn CyclicDevice>>,
    controller: Controller,
    plans: Subscriber<Plan>,
    state: Publisher<ArmState>,
    status: Publisher<Status>,
    /// Last tick index; the supervisor checks that it advances.
    heartbeat: Arc<AtomicU64>,
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
        heartbeat: Arc<AtomicU64>,
        max_consecutive_overruns: u32,
    ) -> Result<Self, IpcError> {
        let node = Node::new(robot, "robotd-rt")?;
        Ok(Self {
            plans: node.subscriber(&PLAN)?,
            state: node.publisher(&STATE)?,
            status: node.publisher(&STATUS)?,
            trigger,
            devices,
            controller,
            heartbeat,
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
            self.heartbeat.store(tick.index, Ordering::Release);
            if now - last_status_ns >= STATUS_PERIOD_NS {
                let _ = self.status.publish(&Status { stats, last: state });
                last_status_ns = now;
            }
        }
    }
}
