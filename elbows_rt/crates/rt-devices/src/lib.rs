//! Devices: setup files, discovery, and the device and transport traits. Drivers live in their own
//! modules here (e.g. `feetech/`), one per protocol; none are implemented yet.
//!
//! - [`setup`]: robot setup files (`hardware/setups/<setup>.toml`), one table per device.
//! - [`serial_ports`]: find a serial device's current port from its USB serial number.
//!
//! Two kinds of device, because they relate to the RT loop differently:
//! - [`CyclicDevice`]: on the control bus, exchanged inside every RT cycle at a fixed phase
//!   (servo buses; later EtherCAT drives, a sim).
//! - [`StreamDevice`]: produces data on its own schedule; owns a thread blocking on its fd and
//!   publishes to topics (cameras, IMUs, an INS).
//!
//! Protocols sit on a [`Transport`], so swapping USB serial for CAN or RS-485 doesn't touch them.

use std::io;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use rt_core::trigger::Tick;

pub mod serial_ports;
pub mod setup;

#[derive(Debug, thiserror::Error)]
pub enum DeviceError {
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error("timed out")]
    Timeout,
    #[error("protocol: {0}")]
    Protocol(String),
    /// The device reports a fault of its own (overtemperature, overload, undervoltage, …).
    #[error("device fault: {0}")]
    Fault(String),
}

/// A byte link to a bus: USB serial now; SocketCAN or RS-485 later.
pub trait Transport: Send {
    fn write_all(&mut self, buf: &[u8]) -> io::Result<()>;
    /// Fill `buf` exactly, or fail with `TimedOut` after `timeout`.
    fn read_exact_timeout(&mut self, buf: &mut [u8], timeout: Duration) -> io::Result<()>;
    /// Drop any unread input, e.g. after a timeout, so the next reply isn't misaligned.
    fn discard_input(&mut self) -> io::Result<()>;
}

/// A device exchanged inside every RT cycle.
///
/// `start` and `stop` run on the supervisor thread and may block and allocate. `read` and `write`
/// run on the RT thread every cycle: bounded time, no allocation, no logging. Each driver exposes its
/// own typed state and command accessors for the controller.
pub trait CyclicDevice: Send {
    fn name(&self) -> &str;
    /// Bring-up: connect, ping, configure (limits, gains, return delay), leave torque off.
    fn start(&mut self) -> Result<(), DeviceError>;
    /// Fill this cycle's measurements.
    fn read(&mut self, tick: &Tick) -> Result<(), DeviceError>;
    /// Send this cycle's commands.
    fn write(&mut self, tick: &Tick) -> Result<(), DeviceError>;
    /// Shutdown: bring to a safe state (hold, then torque off where that is safe).
    fn stop(&mut self) -> Result<(), DeviceError>;
}

/// A device that streams on its own thread and publishes what it reads.
pub trait StreamDevice: Send {
    fn name(&self) -> &str;
    /// Run until `stop` is set or the device fails. Called on the device's own thread, which may
    /// block on the device's fd. Stamp each sample with `CLOCK_MONOTONIC` at capture (or the device's
    /// hardware time mapped onto it).
    fn run(&mut self, stop: &AtomicBool) -> Result<(), DeviceError>;
}
