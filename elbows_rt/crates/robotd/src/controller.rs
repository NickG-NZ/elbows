//! Plan following: accept validated plans, sample the active one each cycle, hold when it ends.
//! Runs on the RT thread: no allocation, no blocking.
//!
//! Not yet: blending between plans (`realtime-control.md` suggests 50–100 ms) and velocity limits.

use rt_core::msgs::{Joints, Mode, Plan, PlanError, PlanLimits};

pub struct Controller {
    limits: PlanLimits,
    mode: Mode,
    plan: Option<Plan>,
    target: Joints,
    rejected: u32,
}

impl Controller {
    pub fn new(limits: PlanLimits, initial: Joints) -> Self {
        Self {
            limits,
            mode: Mode::Hold,
            plan: None,
            target: initial,
            rejected: 0,
        }
    }

    /// Offer a plan from the policy. Invalid plans are counted and ignored; the old one keeps running.
    pub fn offer(&mut self, plan: &Plan, now_ns: u64) -> Result<(), PlanError> {
        if self.mode == Mode::Fault {
            return Ok(());
        }
        if let Err(e) = plan.validate(now_ns, &self.target, &self.limits) {
            self.reject();
            return Err(e);
        }
        self.plan = Some(*plan);
        self.mode = Mode::Running;
        Ok(())
    }

    /// Count a plan that couldn't even be read (corrupt buffer).
    pub fn reject(&mut self) {
        self.rejected = self.rejected.saturating_add(1);
    }

    /// This cycle's target. Past the plan's end, holds its last waypoint (the watchdog: a policy
    /// that stops sending plans leaves the arm holding still).
    pub fn step(&mut self, now_ns: u64) -> Joints {
        if self.mode == Mode::Running
            && let Some(plan) = &self.plan
        {
            self.target = plan.sample(now_ns);
            if now_ns >= plan.end_ns() {
                self.mode = Mode::Hold;
            }
        }
        self.target
    }

    /// Latch Fault: hold the current target and ignore plans until restart.
    pub fn fault(&mut self) {
        self.mode = Mode::Fault;
        self.plan = None;
    }

    pub fn mode(&self) -> Mode {
        self.mode
    }

    pub fn plan_seq(&self) -> u64 {
        self.plan.map_or(0, |p| p.seq)
    }

    pub fn rejected(&self) -> u32 {
        self.rejected
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytemuck::Zeroable;
    use rt_core::msgs::DOF;

    const MS: u64 = 1_000_000;

    fn limits() -> PlanLimits {
        PlanLimits {
            q_min: [-3.0; DOF],
            q_max: [3.0; DOF],
            max_obs_age_ns: 500 * MS,
            max_jump: 0.5,
        }
    }

    fn plan(seq: u64, t0_ns: u64, to: f32) -> Plan {
        let mut p = Plan::zeroed();
        (p.seq, p.t0_ns, p.obs_stamp_ns, p.dt_ns, p.n) = (seq, t0_ns, t0_ns, (10 * MS) as u32, 2);
        p.q[1] = [to; DOF];
        p
    }

    #[test]
    fn follows_then_holds() {
        let mut c = Controller::new(limits(), [0.0; DOF]);
        assert_eq!(c.step(0), [0.0; DOF]);
        c.offer(&plan(1, 100 * MS, 0.2), 100 * MS).unwrap();
        assert_eq!(c.mode(), Mode::Running);
        assert!((c.step(105 * MS)[0] - 0.1).abs() < 1e-6);
        assert_eq!(c.step(120 * MS), [0.2; DOF]);
        assert_eq!(c.mode(), Mode::Hold);
        assert_eq!(c.step(500 * MS), [0.2; DOF]);
    }

    #[test]
    fn rejects_bad_plans_and_keeps_old_one() {
        let mut c = Controller::new(limits(), [0.0; DOF]);
        c.offer(&plan(1, 100 * MS, 0.2), 100 * MS).unwrap();
        let mut bad = plan(2, 100 * MS, 0.2);
        bad.q[0] = [2.0; DOF]; // jump
        assert_eq!(c.offer(&bad, 101 * MS), Err(PlanError::Jump));
        assert_eq!((c.plan_seq(), c.rejected()), (1, 1));
    }

    #[test]
    fn fault_latches() {
        let mut c = Controller::new(limits(), [0.0; DOF]);
        c.fault();
        c.offer(&plan(1, 100 * MS, 0.2), 100 * MS).unwrap();
        assert_eq!(c.mode(), Mode::Fault);
        assert_eq!(c.step(105 * MS), [0.0; DOF]);
    }
}
