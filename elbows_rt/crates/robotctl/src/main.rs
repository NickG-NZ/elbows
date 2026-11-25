//! Operator CLI. Device discovery now; robotd status, enable/stop and calibration later.
//!
//! ```text
//! robotctl devices list                          connected serial ports and their USB serial numbers
//! robotctl devices port --setup FILE <device>    current port of a device in a robot setup file
//! ```
//!
//! e.g. for LeRobot bring-up:
//! `lerobot-calibrate … --robot.port=$(robotctl devices port --setup hardware/setups/so101.toml arm)`

use std::process::ExitCode;

use rt_devices::serial_ports;
use rt_devices::setup::Setup;

const USAGE: &str = "usage:
  robotctl devices list
  robotctl devices port --setup FILE DEVICE";

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
        _ => Err(USAGE.to_string()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("robotctl: {e}");
            ExitCode::FAILURE
        }
    }
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
