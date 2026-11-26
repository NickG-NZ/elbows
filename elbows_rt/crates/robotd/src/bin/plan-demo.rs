//! A stand-in policy process: reads robotd's state, sends 1 s sine-wave plans at 10 Hz.
//! Exercises the full path (topics, validation, sampling, hold) without a model.
//!
//! The same code either way: only the node's `Link` differs.
//!
//! Usage:
//! - `plan-demo --robot so101` — a local policy, over iceoryx2 shared memory.
//! - `plan-demo --robot so101 --remote [ENDPOINT]` — a remote policy, over Zenoh, as one on another
//!   machine would be (default endpoint `tcp/127.0.0.1:7447`).
//!
//! robotd must be enabled (`robotctl --robot so101 enable`). Stop this and robotd drops to Hold.
//!
//! Plan times are on the *robot's* clock, which a remote machine doesn't share: the robot's "now" is
//! estimated as the last state's stamp plus the time since that state arrived.

use std::f64::consts::TAU;
use std::time::{Duration, Instant};

use bytemuck::Zeroable;
use rt_core::msgs::{ArmState, DOF, Mode, N_MAX, Plan};
use rt_core::topics::{PLAN, STATE};
use rt_node::{Link, Node};

const PLAN_HZ: f64 = 10.0;
const DT_NS: u64 = 20_000_000;
const WAYPOINTS: usize = 50;
const LEAD_NS: u64 = 20_000_000;
const SINE_HZ: f64 = 0.5;
const RAMP_S: f64 = 2.0;
/// Don't plan from robot state older than this (robotd not running).
const MAX_STATE_AGE: Duration = Duration::from_millis(200);

fn main() -> Result<(), String> {
    let usage = "usage: plan-demo --robot NAME [--remote [ENDPOINT]] [--amp RAD]";
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (mut robot, mut remote, mut amp) = (None, None, 0.3);
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--robot" => {
                robot = args.get(i + 1).cloned();
                i += 1;
            }
            "--amp" => {
                amp = args
                    .get(i + 1)
                    .and_then(|v| v.parse().ok())
                    .ok_or("--amp needs a number")?;
                i += 1;
            }
            "--remote" => match args.get(i + 1).filter(|a| !a.starts_with("--")) {
                Some(ep) => {
                    remote = Some(ep.clone());
                    i += 1;
                }
                None => remote = Some(rt_node::DEFAULT_ENDPOINT.to_string()),
            },
            _ => return Err(usage.into()),
        }
        i += 1;
    }
    let robot = robot.ok_or(usage)?;
    let link = remote.clone().map_or(Link::Local, Link::Remote);
    let node = Node::new(&robot, "policy", link).map_err(|e| e.to_string())?;
    let plans = node.publisher(&PLAN).map_err(|e| {
        format!("{e} (another policy, local or remote, already publishes {robot}/plan)")
    })?;
    let states = node.subscriber(&STATE).map_err(|e| e.to_string())?;
    eprintln!(
        "plan-demo: {} policy for {robot}",
        if remote.is_some() { "remote" } else { "local" }
    );

    let mut start_ns: Option<u64> = None;
    let mut seq = 0;
    let mut last: Option<(ArmState, Instant)> = None;
    let mut last_print = Instant::now() - Duration::from_secs(2);
    loop {
        std::thread::sleep(Duration::from_secs_f64(1.0 / PLAN_HZ));
        if let Ok(Some(s)) = states.latest() {
            last = Some((s, Instant::now()));
        }
        let Some((state, received)) = last.filter(|(_, at)| at.elapsed() < MAX_STATE_AGE) else {
            continue;
        };
        let robot_now = state.stamp_ns + received.elapsed().as_nanos() as u64;
        let start = *start_ns.get_or_insert(robot_now);

        let t0_ns = robot_now + LEAD_NS;
        let scale = ((t0_ns - start) as f64 / 1e9 / RAMP_S).min(1.0) * amp;
        let mut plan = Plan::zeroed();
        seq += 1;
        (plan.seq, plan.t0_ns, plan.obs_stamp_ns, plan.dt_ns, plan.n) =
            (seq, t0_ns, state.stamp_ns, DT_NS as u32, WAYPOINTS as u32);
        for (k, q) in plan.q.iter_mut().take(WAYPOINTS.min(N_MAX)).enumerate() {
            let t = (t0_ns - start + k as u64 * DT_NS) as f64 / 1e9;
            for (j, qj) in q.iter_mut().take(DOF - 1).enumerate() {
                *qj = (scale * (TAU * SINE_HZ * t + j as f64).sin()) as f32;
            }
        }
        let _ = plans.publish(&plan);

        if last_print.elapsed() >= Duration::from_secs(1) {
            let mode = Mode::from_u32(state.mode).map_or("?", Mode::name);
            eprintln!(
                "plan-demo: sent {seq}, robot {mode} following {} (rejected {}), q_cmd[0] {:+.3}",
                state.plan_seq, state.plans_rejected, state.q_cmd[0]
            );
            last_print = Instant::now();
        }
    }
}
