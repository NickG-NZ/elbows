//! Messages crossing process boundaries. Fixed size, `#[repr(C)]`, `Pod`, no pointers.
//! See `docs/realtime-control.md` for the plan's semantics.

use bytemuck::{Pod, Zeroable};
use iceoryx2_bb_derive_macros::ZeroCopySend;
use iceoryx2_bb_elementary_traits::zero_copy_send::ZeroCopySend;

use crate::stats::CycleStats;

/// Joints per arm: SO-101 has 5 plus the gripper.
pub const DOF: usize = 6;
/// Waypoints per plan.
pub const N_MAX: usize = 64;

pub type Joints = [f32; DOF];

/// A timed action chunk: waypoint `k` applies at `t0_ns + k * dt_ns` on the robot's
/// `CLOCK_MONOTONIC`. Plans are sampled at the current time, never consumed as a queue.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable, ZeroCopySend)]
#[type_name("elbows.Plan")]
pub struct Plan {
    /// Set by the policy; increases with each plan.
    pub seq: u64,
    pub t0_ns: u64,
    /// `stamp_ns` of the robot state this plan was computed from.
    pub obs_stamp_ns: u64,
    pub dt_ns: u32,
    /// Valid waypoints, 1..=N_MAX.
    pub n: u32,
    pub q: [Joints; N_MAX],
}

#[derive(Debug, Clone, Copy)]
pub struct PlanLimits {
    pub q_min: Joints,
    pub q_max: Joints,
    /// Reject a plan computed from an observation older than this.
    pub max_obs_age_ns: u64,
    /// Reject a plan whose target at "now" is further than this from the current target (per joint).
    pub max_jump: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PlanError {
    #[error("n must be 1..=N_MAX")]
    Length,
    #[error("dt must be positive")]
    Dt,
    #[error("non-finite waypoint")]
    NonFinite,
    #[error("waypoint outside joint limits")]
    Limits,
    #[error("observation too old (or from the future)")]
    Stale,
    #[error("jump from current target too large")]
    Jump,
    #[error("plan already over")]
    Expired,
}

impl Plan {
    pub fn waypoints(&self) -> &[Joints] {
        &self.q[..(self.n as usize).min(N_MAX)]
    }

    /// Time of the last waypoint.
    pub fn end_ns(&self) -> u64 {
        self.t0_ns + self.n.saturating_sub(1) as u64 * self.dt_ns as u64
    }

    /// Everything the controller checks before accepting a plan from another process.
    pub fn validate(
        &self,
        now_ns: u64,
        current: &Joints,
        lim: &PlanLimits,
    ) -> Result<(), PlanError> {
        if self.n == 0 || self.n as usize > N_MAX {
            return Err(PlanError::Length);
        }
        if self.dt_ns == 0 {
            return Err(PlanError::Dt);
        }
        for q in self.waypoints() {
            if q.iter().any(|x| !x.is_finite()) {
                return Err(PlanError::NonFinite);
            }
            if (0..DOF).any(|j| q[j] < lim.q_min[j] || q[j] > lim.q_max[j]) {
                return Err(PlanError::Limits);
            }
        }
        if self.obs_stamp_ns > now_ns || now_ns - self.obs_stamp_ns > lim.max_obs_age_ns {
            return Err(PlanError::Stale);
        }
        if self.n > 1 && now_ns > self.end_ns() {
            return Err(PlanError::Expired);
        }
        let target = self.sample(now_ns);
        if (0..DOF).any(|j| (target[j] - current[j]).abs() > lim.max_jump) {
            return Err(PlanError::Jump);
        }
        Ok(())
    }

    /// Target at time `t_ns`: linear between waypoints, held at the ends. Call only after `validate`.
    pub fn sample(&self, t_ns: u64) -> Joints {
        let w = self.waypoints();
        if t_ns <= self.t0_ns || w.len() == 1 {
            return w[0];
        }
        let pos = (t_ns - self.t0_ns) as f64 / self.dt_ns as f64;
        let k = pos as usize;
        if k + 1 >= w.len() {
            return w[w.len() - 1];
        }
        let a = (pos - k as f64) as f32;
        std::array::from_fn(|j| w[k][j] + a * (w[k + 1][j] - w[k][j]))
    }
}

/// What the RT thread is doing.
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Holding the last target. The start state, and where a finished or stale plan ends up.
    Hold = 0,
    /// Following a plan.
    Running = 1,
    /// Latched after a fault (e.g. repeated overruns, device error). Needs a restart for now.
    Fault = 2,
}

impl Mode {
    pub fn from_u32(v: u32) -> Option<Self> {
        [Mode::Hold, Mode::Running, Mode::Fault]
            .into_iter()
            .find(|m| *m as u32 == v)
    }
}

/// Published by the RT thread every cycle.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable, ZeroCopySend)]
#[type_name("elbows.ArmState")]
pub struct ArmState {
    /// `CLOCK_MONOTONIC` time of the cycle this state was measured in. Policies echo it as
    /// `Plan::obs_stamp_ns`.
    pub stamp_ns: u64,
    pub cycle: u64,
    /// `seq` of the plan being followed, 0 if none.
    pub plan_seq: u64,
    /// A [`Mode`] as u32.
    pub mode: u32,
    pub plans_rejected: u32,
    pub q_cmd: Joints,
    pub q_meas: Joints,
}

/// What the RT thread reports about itself, ~10 Hz.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable, ZeroCopySend)]
#[type_name("elbows.Status")]
pub struct Status {
    pub stats: CycleStats,
    pub last: ArmState,
}

#[cfg(test)]
mod tests {
    use super::*;

    const MS: u64 = 1_000_000;

    fn limits() -> PlanLimits {
        PlanLimits {
            q_min: [-3.0; DOF],
            q_max: [3.0; DOF],
            max_obs_age_ns: 500 * MS,
            max_jump: 0.5,
        }
    }

    fn ramp(t0_ns: u64) -> Plan {
        let mut p = Plan::zeroed();
        p.t0_ns = t0_ns;
        p.obs_stamp_ns = t0_ns;
        p.dt_ns = (10 * MS) as u32;
        p.n = 3;
        for (k, q) in p.q.iter_mut().take(3).enumerate() {
            *q = [k as f32 * 0.1; DOF];
        }
        p
    }

    #[test]
    fn samples_by_time() {
        let p = ramp(1000 * MS);
        assert_eq!(p.sample(0)[0], 0.0);
        assert!((p.sample(1005 * MS)[0] - 0.05).abs() < 1e-6);
        assert!((p.sample(1015 * MS)[0] - 0.15).abs() < 1e-6);
        assert_eq!(p.sample(5000 * MS)[0], 0.2);
        assert_eq!(p.end_ns(), 1020 * MS);
    }

    #[test]
    fn validates() {
        let now = 1000 * MS;
        let cur = [0.0; DOF];
        assert_eq!(ramp(now).validate(now, &cur, &limits()), Ok(()));
        let check = |f: fn(&mut Plan)| {
            let mut p = ramp(now);
            f(&mut p);
            p.validate(now, &cur, &limits())
        };
        assert_eq!(check(|p| p.n = 0), Err(PlanError::Length));
        assert_eq!(check(|p| p.n = N_MAX as u32 + 1), Err(PlanError::Length));
        assert_eq!(check(|p| p.dt_ns = 0), Err(PlanError::Dt));
        assert_eq!(check(|p| p.q[1][2] = f32::NAN), Err(PlanError::NonFinite));
        assert_eq!(check(|p| p.q[2][0] = 9.0), Err(PlanError::Limits));
        assert_eq!(check(|p| p.obs_stamp_ns = 0), Err(PlanError::Stale));
        assert_eq!(check(|p| p.q[0] = [0.9; DOF]), Err(PlanError::Jump));
        assert_eq!(
            ramp(now).validate(now + 100 * MS, &cur, &limits()),
            Err(PlanError::Expired)
        );
    }
}
