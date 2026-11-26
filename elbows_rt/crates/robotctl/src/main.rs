//! Operator CLI.
//!
//! ```text
//! robotctl --robot NAME status|enable|hold|stop [--connect ENDPOINT] [--json]
//! robotctl devices list                          connected serial ports and their USB serial numbers
//! robotctl devices port --setup DIR <device>     current port of a device in a robot setup
//! ```
//!
//! Commands go to robotd over Zenoh (default endpoint `tcp/127.0.0.1:7447`); robotd's RT thread
//! decides whether to apply them, and its answer is printed. Exits non-zero if refused or unreachable.
//!
//! e.g. for LeRobot bring-up:
//! `lerobot-calibrate … --robot.port=$(robotctl devices port --setup hardware/setups/so101 arm)`

use std::process::ExitCode;

use rt_core::services::{COMMANDS, Reply};
use rt_devices::serial_ports;
use rt_devices::setup::Setup;
use rt_node::{Link, Node};

const USAGE: &str = "usage:
  robotctl --robot NAME status|enable|hold|stop [--connect ENDPOINT] [--json]
  robotctl devices list
  robotctl devices port --setup DIR DEVICE";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let result = match args.as_slice() {
        ["devices", "list"] => list(),
        ["devices", "port", "--setup", setup, device]
        | ["devices", "port", device, "--setup", setup] => port(setup, device),
        ["-h" | "--help" | "help"] => {
            println!("{USAGE}");
            Ok(())
        }
        rest => command(rest),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("robotctl: {e}");
            ExitCode::FAILURE
        }
    }
}

fn command(args: &[&str]) -> Result<(), String> {
    let (mut robot, mut endpoint, mut json, mut cmd) =
        (None, rt_node::DEFAULT_ENDPOINT, false, None);
    let mut it = args.iter();
    while let Some(&a) = it.next() {
        match a {
            "--robot" => robot = it.next().copied(),
            "--connect" => endpoint = it.next().copied().ok_or("--connect needs an endpoint")?,
            "--json" => json = true,
            _ => {
                let svc = COMMANDS.into_iter().find(|s| s.name == a);
                cmd = Some(svc.ok_or(format!("unknown command {a:?}\n{USAGE}"))?);
            }
        }
    }
    let robot = robot.ok_or(format!("--robot is required\n{USAGE}"))?;
    let cmd = cmd.ok_or(USAGE)?;
    // Remote even on the robot: robotctl only calls services, which go over Zenoh either way.
    let node = Node::new(robot, "robotctl", Link::Remote(endpoint.to_string()))
        .map_err(|e| e.to_string())?;
    let reply: Reply = node.call(cmd, &()).map_err(|e| e.to_string())?;
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&reply).expect("reply serializes")
        );
    } else {
        print_reply(&reply);
    }
    if reply.ok { Ok(()) } else { Err(reply.message) }
}

fn print_reply(r: &Reply) {
    if !r.message.is_empty() && r.ok {
        println!("{}", r.message);
    }
    let s = &r.status;
    println!("mode      {}", s.mode);
    println!(
        "plan      seq {}, {} rejected",
        s.plan_seq, s.plans_rejected
    );
    println!("cycles    {} ({} missed)", s.cycles, s.missed);
    println!(
        "latency   p99 <{} µs, max {} µs; exec p99 <{} µs",
        s.latency_p99_us, s.latency_max_us, s.exec_p99_us
    );
    println!(
        "nodes     {}",
        if s.nodes.is_empty() {
            "none".into()
        } else {
            s.nodes.join(", ")
        }
    );
}

fn list() -> Result<(), String> {
    for p in serial_ports::list().map_err(|e| e.to_string())? {
        let serial = p.usb_serial.as_deref().unwrap_or("-");
        println!("{}\tserial={serial}\t{}", p.path, p.description);
    }
    Ok(())
}

fn port(setup: &str, device: &str) -> Result<(), String> {
    let setup = Setup::load(setup).map_err(|e| e.to_string())?;
    let serial = setup
        .device_str(device, "usb_serial")
        .map_err(|e| e.to_string())?;
    println!(
        "{}",
        serial_ports::find_port(serial).map_err(|e| e.to_string())?
    );
    Ok(())
}
