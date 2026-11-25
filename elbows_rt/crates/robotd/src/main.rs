//! `robotd`: the one process that owns the robot's hardware.
//!
//! The main thread is the supervisor: config, signals, health and status. It spawns the RT thread,
//! which runs the control loop. Policies talk to it through topics (`rt_core::topics`), namespaced
//! by robot name:
//!
//! - `<robot>/plan`   — the policy publishes, the RT thread subscribes
//! - `<robot>/state`  — the RT thread publishes every cycle; policies, loggers, viewers subscribe
//! - `<robot>/status` — the RT thread publishes ~10 Hz; the supervisor (and later `robotctl`) subscribe
//!
//! Usage: `robotd --robot so101 [--setup hardware/setups/so101.toml] [--rate-hz 200]`

mod config;
mod controller;
mod rt_loop;

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use rt_core::msgs::{DOF, Mode, Status};
use rt_core::rt::{RT_STACK_BYTES, setup_current_thread};
use rt_core::topics::STATUS;
use rt_core::trigger::ClockTrigger;
use rt_ipc::Node;

use crate::config::Config;
use crate::controller::Controller;
use crate::rt_loop::RtLoop;

const SUPERVISOR_PERIOD: Duration = Duration::from_millis(100);
const REPORT_PERIOD: Duration = Duration::from_secs(1);
const STALL_AFTER: Duration = Duration::from_millis(500);

struct Args {
    robot: String,
    setup: Option<PathBuf>,
    rate_hz: Option<f64>,
}

fn parse_args() -> Result<Args, String> {
    let mut args = std::env::args().skip(1);
    let (mut robot, mut setup, mut rate_hz) = (None, None, None);
    while let Some(a) = args.next() {
        let mut val = || args.next().ok_or(format!("{a} needs a value"));
        match a.as_str() {
            "--robot" => robot = Some(val()?),
            "--setup" => setup = Some(PathBuf::from(val()?)),
            "--rate-hz" => rate_hz = Some(val()?.parse().map_err(|e| format!("--rate-hz: {e}"))?),
            "-h" | "--help" => return Err("help".into()),
            _ => return Err(format!("unknown argument {a}")),
        }
    }
    let robot = robot.ok_or("--robot is required")?;
    if robot.is_empty()
        || !robot
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return Err(format!(
            "bad robot name {robot:?} (letters, digits, _ and - only)"
        ));
    }
    Ok(Args {
        robot,
        setup,
        rate_hz,
    })
}

const USAGE: &str = "usage: robotd --robot NAME [--setup FILE] [--rate-hz HZ]";

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("robotd: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let args = parse_args().map_err(|e| format!("{e}\n{USAGE}"))?;
    let mut cfg = Config::load(args.setup.as_deref())?;
    if let Some(hz) = args.rate_hz {
        cfg.rate_hz = hz;
    }

    let term = Arc::new(AtomicBool::new(false));
    for sig in [signal_hook::consts::SIGINT, signal_hook::consts::SIGTERM] {
        signal_hook::flag::register(sig, term.clone())
            .map_err(|e| format!("signal handler: {e}"))?;
    }

    // The RT thread: real-time setup first, then its node and ports, then the loop.
    let heartbeat = Arc::new(AtomicU64::new(0));
    let rt_stop = Arc::new(AtomicBool::new(false));
    let rt_cfg = cfg.rt_config();
    let (setup_tx, setup_rx) = std::sync::mpsc::channel::<(Vec<String>, Result<(), String>)>();
    let rt_thread = {
        let (robot, cfg, rt_cfg, heartbeat, stop) = (
            args.robot.clone(),
            cfg.clone(),
            rt_cfg.clone(),
            heartbeat.clone(),
            rt_stop.clone(),
        );
        std::thread::Builder::new()
            .name("rt".into())
            .stack_size(RT_STACK_BYTES)
            .spawn(move || {
                let failed = setup_current_thread(&rt_cfg);
                if rt_cfg.required && !failed.is_empty() {
                    let _ = setup_tx.send((
                        failed,
                        Err("RT setup failed and robotd.rt.required is set".into()),
                    ));
                    return;
                }
                let built = RtLoop::new(
                    &robot,
                    Box::new(ClockTrigger::from_hz(cfg.rate_hz)),
                    Vec::new(), // drivers (Feetech first) go here, after `start()`
                    Controller::new(cfg.plan_limits(), [0.0; DOF]),
                    heartbeat,
                    cfg.max_consecutive_overruns,
                );
                match built {
                    Ok(rt_loop) => {
                        let _ = setup_tx.send((failed, Ok(())));
                        rt_loop.run(&stop);
                    }
                    Err(e) => {
                        let hint = format!("{e} (is another robotd running for {robot}?)");
                        let _ = setup_tx.send((failed, Err(hint)));
                    }
                }
            })
            .map_err(|e| format!("spawn RT thread: {e}"))?
    };

    let (failed, built) = setup_rx
        .recv()
        .map_err(|_| "RT thread died during setup".to_string())?;
    for f in &failed {
        eprintln!("robotd: RT setup: {f}");
    }
    if let Err(e) = built {
        let _ = rt_thread.join();
        return Err(e);
    }
    eprintln!("robotd: {} running at {} Hz", args.robot, cfg.rate_hz);

    // Supervisor loop.
    let node = Node::new(&args.robot, "robotd").map_err(|e| e.to_string())?;
    let status_rx = node.subscriber(&STATUS).map_err(|e| e.to_string())?;
    let mut last_hb = (heartbeat.load(Ordering::Acquire), Instant::now());
    let mut stalled = false;
    let mut last_report = Instant::now();
    let mut status: Option<Status> = None;
    while !term.load(Ordering::Relaxed) && !rt_thread.is_finished() {
        std::thread::sleep(SUPERVISOR_PERIOD);
        let hb = heartbeat.load(Ordering::Acquire);
        if hb != last_hb.0 {
            last_hb = (hb, Instant::now());
            stalled = false;
        } else if !stalled && last_hb.1.elapsed() > STALL_AFTER {
            eprintln!("robotd: RT thread stalled at tick {hb}");
            stalled = true;
        }
        if let Ok(Some(s)) = status_rx.latest() {
            status = Some(s);
        }
        if last_report.elapsed() >= REPORT_PERIOD {
            if let Some(s) = &status {
                report(s);
            }
            last_report = Instant::now();
        }
    }

    rt_stop.store(true, Ordering::Relaxed);
    rt_thread
        .join()
        .map_err(|_| "RT thread panicked".to_string())?;
    eprintln!("robotd: stopped");
    Ok(())
}

fn report(s: &Status) {
    let mode = Mode::from_u32(s.last.mode).map_or("?".into(), |m| format!("{m:?}"));
    let st = &s.stats;
    eprintln!(
        "robotd: {mode:<7} plan {:<5} rejected {:<3} | cycles {} missed {} | latency p99 <{}µs max {}µs | exec p99 <{}µs",
        s.last.plan_seq,
        s.last.plans_rejected,
        st.cycles,
        st.missed,
        st.latency.quantile_us(0.99),
        st.latency.max_ns / 1000,
        st.exec.quantile_us(0.99),
    );
}
