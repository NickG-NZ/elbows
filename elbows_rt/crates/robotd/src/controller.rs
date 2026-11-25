//! Modes and plan following: accept validated plans, sample the active one each cycle, hold when it
//! ends. Runs on the RT thread: no allocation, no blocking.
//!
//! ```text
//!            enable              valid plan
//! Disabled ─────────▶ Hold ◀──────────────────▶ Running
//!     ▲                │   plan ends / `hold`      │
//!     └──── `stop` ────┴───────────────────────────┘
//! any mode ── fault (overruns, device error) ──▶ Fault (latched until restart)
//! ```
//!
//! Not yet: blending between plans (`realtime-control.md` suggests 50–100 ms) and velocity limits.

use rt_core::msgs::{Joints, Mode, Plan, PlanError, PlanLimits};

/// A mode change requested by the supervisor. The controller decides whether to apply it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModeCommand {
    Enable,
    Hold,
    Stop,
}

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
            mode: Mode::Disabled,
            plan: None,
            target: initial,
            rejected: 0,
        }
    }

    /// Apply a mode command, or say why not. `measured` is the current measured pose: enabling holds
    /// where the arm actually is, so it never jumps.
    pub fn command(&mut self, cmd: ModeCommand, measured: &Joints) -> Result<(), &'static str> {
        match (cmd, self.mode) {
            (_, Mode::Fault) => Err("fault latched; restart robotd"),
            (ModeCommand::Enable, Mode::Disabled) => {
                self.target = *measured;
                self.mode = Mode::Hold;
                Ok(())
            }
            (ModeCommand::Enable, _) => Ok(()),
            (ModeCommand::Hold, Mode::Disabled) => Err("disabled; enable first"),
            (ModeCommand::Hold, _) => {
                self.plan = None;
                self.mode = Mode::Hold;
                Ok(())
            }
            (ModeCommand::Stop, _) => {
                self.plan = None;
                self.mode = Mode::Disabled;
                Ok(())
            }
        }
    }

    /// Offer a plan from the policy. Invalid plans are counted and ignored; the old one keeps running.
    pub fn offer(&mut self, plan: &Plan, now_ns: u64) -> Result<(), PlanError> {
        match self.mode {
            Mode::Fault => return Ok(()), // latched: ignore quietly
            Mode::Disabled => {
                self.reject();
                return Err(PlanError::NotEnabled);
            }
            Mode::Hold | Mode::Running => {}
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

    /// Latch Fault: hold the current target and ignore plans and commands until restart.
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
    const ZERO: Joints = [0.0; DOF];

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

    fn enabled() -> Controller {
        let mut c = Controller::new(limits(), ZERO);
        c.command(ModeCommand::Enable, &ZERO).unwrap();
        c
    }

    #[test]
    fn starts_disabled_and_refuses_plans() {
        let mut c = Controller::new(limits(), ZERO);
        assert_eq!(c.mode(), Mode::Disabled);
        assert_eq!(
            c.offer(&plan(1, 100 * MS, 0.2), 100 * MS),
            Err(PlanError::NotEnabled)
        );
        assert_eq!(
            c.command(ModeCommand::Hold, &ZERO),
            Err("disabled; enable first")
        );
        assert_eq!(c.rejected(), 1);
    }

    #[test]
    fn enable_holds_the_measured_pose() {
        let mut c = Controller::new(limits(), ZERO);
        c.command(ModeCommand::Enable, &[0.1; DOF]).unwrap();
        assert_eq!((c.mode(), c.step(0)), (Mode::Hold, [0.1; DOF]));
    }

    #[test]
    fn follows_then_holds() {
        let mut c = enabled();
        c.offer(&plan(1, 100 * MS, 0.2), 100 * MS).unwrap();
        assert_eq!(c.mode(), Mode::Running);
        assert!((c.step(105 * MS)[0] - 0.1).abs() < 1e-6);
        assert_eq!(c.step(120 * MS), [0.2; DOF]);
        assert_eq!(c.mode(), Mode::Hold);
        assert_eq!(c.step(500 * MS), [0.2; DOF]);
    }

    #[test]
    fn hold_and_stop_drop_the_plan() {
        let mut c = enabled();
        c.offer(&plan(1, 100 * MS, 0.2), 100 * MS).unwrap();
        c.step(105 * MS);
        c.command(ModeCommand::Hold, &ZERO).unwrap();
        let held = c.step(110 * MS);
        assert_eq!((c.mode(), c.step(200 * MS)), (Mode::Hold, held));
        c.command(ModeCommand::Stop, &ZERO).unwrap();
        assert_eq!(c.mode(), Mode::Disabled);
    }

    #[test]
    fn rejects_bad_plans_and_keeps_old_one() {
        let mut c = enabled();
        c.offer(&plan(1, 100 * MS, 0.2), 100 * MS).unwrap();
        let mut bad = plan(2, 100 * MS, 0.2);
        bad.q[0] = [2.0; DOF]; // jump
        assert_eq!(c.offer(&bad, 101 * MS), Err(PlanError::Jump));
        assert_eq!((c.plan_seq(), c.rejected()), (1, 1));
    }

    #[test]
    fn fault_latches() {
        let mut c = enabled();
        c.fault();
        c.offer(&plan(1, 100 * MS, 0.2), 100 * MS).unwrap();
        assert_eq!(c.mode(), Mode::Fault);
        assert_eq!(c.step(105 * MS), ZERO);
        assert!(c.command(ModeCommand::Enable, &ZERO).is_err());
        assert!(c.command(ModeCommand::Stop, &ZERO).is_err());
    }
}
