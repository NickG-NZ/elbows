//! A stand-in policy process: reads robotd's state, sends 1 s sine-wave plans at 10 Hz.
//! Exercises the full path (topics, validation, sampling, hold) without a model.
//!
//! Usage: `plan-demo --robot so101 [--amp 0.3]`. Stop it and robotd drops back to Hold.

use std::f64::consts::TAU;
use std::time::Duration;

use bytemuck::Zeroable;
use rt_core::msgs::{ArmState, DOF, Mode, N_MAX, Plan};
use rt_core::time::monotonic_ns;
use rt_core::topics::{PLAN, STATE};
use rt_ipc::Node;

const PLAN_HZ: f64 = 10.0;
const DT_NS: u64 = 20_000_000;
const WAYPOINTS: usize = 50;
const LEAD_NS: u64 = 20_000_000;
const SINE_HZ: f64 = 0.5;
const RAMP_S: f64 = 2.0;
/// Don't plan from robot state older than this (robotd not running).
const MAX_STATE_AGE_NS: u64 = 200_000_000;

fn main() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let (mut robot, mut amp) = (None, 0.3);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--robot" => robot = args.next(),
            "--amp" => {
                amp = args
                    .next()
                    .and_then(|v| v.parse().ok())
                    .ok_or("--amp needs a number")?
            }
            _ => return Err("usage: plan-demo --robot NAME [--amp RAD]".into()),
        }
    }
    let robot = robot.ok_or("--robot is required")?;
    let node = Node::new(&robot, "plan-demo").map_err(|e| e.to_string())?;
    let plans = node.publisher(&PLAN).map_err(|e| e.to_string())?;
    let states = node.subscriber(&STATE).map_err(|e| e.to_string())?;
    eprintln!("plan-demo: publishing {}", node.topic_name(&PLAN));

    let start_ns = monotonic_ns();
    let mut seq = 0;
    let mut last: Option<ArmState> = None;
    let mut last_print = 0;
    loop {
        std::thread::sleep(Duration::from_secs_f64(1.0 / PLAN_HZ));
        if let Ok(Some(s)) = states.latest() {
            last = Some(s);
        }
        let now = monotonic_ns();
        let Some(state) = last.filter(|s| now.saturating_sub(s.stamp_ns) < MAX_STATE_AGE_NS) else {
            continue;
        };

        let t0_ns = now + LEAD_NS;
        let scale = ((t0_ns - start_ns) as f64 / 1e9 / RAMP_S).min(1.0) * amp;
        let mut plan = Plan::zeroed();
        seq += 1;
        (plan.seq, plan.t0_ns, plan.obs_stamp_ns, plan.dt_ns, plan.n) =
            (seq, t0_ns, state.stamp_ns, DT_NS as u32, WAYPOINTS as u32);
        for (k, q) in plan.q.iter_mut().take(WAYPOINTS.min(N_MAX)).enumerate() {
            let t = (t0_ns - start_ns + k as u64 * DT_NS) as f64 / 1e9;
            for (j, qj) in q.iter_mut().take(DOF - 1).enumerate() {
                *qj = (scale * (TAU * SINE_HZ * t + j as f64).sin()) as f32;
            }
        }
        let _ = plans.publish(&plan);

        if now - last_print > 1_000_000_000 {
            let mode = Mode::from_u32(state.mode).map_or("?".into(), |m| format!("{m:?}"));
            eprintln!(
                "plan-demo: sent {seq}, robot {mode} following {} (rejected {}), q_cmd[0] {:+.3}",
                state.plan_seq, state.plans_rejected, state.q_cmd[0]
            );
            last_print = now;
        }
    }
}
