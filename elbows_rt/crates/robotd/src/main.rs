//! `robotd`: the one process that owns the robot's hardware.
//!
//! Two threads:
//! - **RT thread**: the control loop. Talks to policies through local topics (`rt_core::topics`,
//!   namespaced by robot name) and to the supervisor in-process. Starts **disabled**.
//! - **Supervisor** (the main thread): robotd's only external interface. Commands and sessions
//!   over Zenoh (`rt-net`), the bridge for remote policies and viewers, health.
//!
//! Local topics: `<robot>/plan` (policy → RT), `<robot>/state` (RT → anyone, every cycle),
//! `<robot>/status` (RT → anyone, ~10 Hz). Zenoh keys: see `rt-net`.
//!
//! Usage: `robotd --robot so101 [--setup hardware/setups/so101] [--rate-hz 200]`, then
//! `robotctl --robot so101 enable`.

mod bridge;
mod config;
mod controller;
mod rt_loop;
mod supervisor;

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::sync_channel;

use rt_core::msgs::DOF;
use rt_core::rt::{RT_STACK_BYTES, setup_current_thread};
use rt_core::trigger::ClockTrigger;
use rt_net::NetConfig;

use crate::bridge::Bridge;
use crate::config::Config;
use crate::controller::Controller;
use crate::rt_loop::{RtLoop, SupervisorLink};
use crate::supervisor::{RtLink, Supervisor};

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

const USAGE: &str = "usage: robotd --robot NAME [--setup DIR] [--rate-hz HZ]";

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

    // In-process links between the supervisor and the RT thread, allocated up front.
    let (commands_tx, commands_rx) = sync_channel(16);
    let (outcomes_tx, outcomes_rx) = sync_channel(16);
    let (status_tx, status_rx) = sync_channel(4);
    let heartbeat = Arc::new(AtomicU64::new(0));
    let rt_link = SupervisorLink {
        commands: commands_rx,
        outcomes: outcomes_tx,
        status: status_tx,
        heartbeat: heartbeat.clone(),
    };

    // The RT thread: real-time setup first, then its node and ports, then the loop.
    let rt_stop = Arc::new(AtomicBool::new(false));
    let rt_cfg = cfg.rt_config();
    let (setup_tx, setup_rx) = std::sync::mpsc::channel::<(Vec<String>, Result<(), String>)>();
    let rt_thread = {
        let (robot, cfg, rt_cfg, stop) = (
            args.robot.clone(),
            cfg.clone(),
            rt_cfg.clone(),
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
                    rt_link,
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
    let stop_rt = |rt_thread: std::thread::JoinHandle<()>| {
        rt_stop.store(true, Ordering::Relaxed);
        rt_thread
            .join()
            .map_err(|_| "RT thread panicked".to_string())
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

    // The supervisor, on this thread.
    let net_cfg = NetConfig {
        listen: cfg.net.listen.clone(),
        connect: Vec::new(),
    };
    let supervisor = rt_net::open(&net_cfg)
        .map_err(|e| e.to_string())
        .and_then(|net| {
            let rt = RtLink {
                commands: commands_tx,
                outcomes: outcomes_rx,
                status: status_rx,
                heartbeat,
            };
            let bridge = Bridge::from_names(&args.robot, &net, &cfg.net.out, &cfg.net.inbound)?;
            Supervisor::new(&args.robot, rt, net, bridge)
        });
    let mut supervisor = match supervisor {
        Ok(s) => s,
        Err(e) => {
            stop_rt(rt_thread)?;
            return Err(e);
        }
    };
    eprintln!(
        "robotd: {} at {} Hz, disabled; commands on {:?} (robotctl --robot {} enable)",
        args.robot, cfg.rate_hz, cfg.net.listen, args.robot
    );
    supervisor.run(&term, || rt_thread.is_finished());

    stop_rt(rt_thread)?;
    eprintln!("robotd: stopped");
    Ok(())
}
