//! Make the calling thread real-time: lock memory, pin to a core, `SCHED_FIFO`, prefault the stack.
//!
//! Only Linux can do this. Elsewhere (dev machines) every requested step reports as not applied,
//! and the caller decides whether that's fatal via [`RtConfig::required`].

#[derive(Debug, Clone, Default)]
pub struct RtConfig {
    /// Core to pin to. Pair with `isolcpus=`/`nohz_full=` on the kernel command line.
    pub cpu: Option<usize>,
    /// `SCHED_FIFO` priority, 1–99. Needs CAP_SYS_NICE or an rtprio limit.
    pub priority: Option<i32>,
    /// `mlockall(MCL_CURRENT | MCL_FUTURE)`. Needs CAP_IPC_LOCK or a memlock limit.
    pub lock_memory: bool,
    /// Fail instead of warn if a step can't be applied. Set on the robot, not on dev machines.
    pub required: bool,
}

/// Stack bytes to touch up front, so the loop never page-faults growing its stack.
pub const PREFAULT_STACK_BYTES: usize = 256 * 1024;
/// Stack size to spawn the RT thread with; must exceed [`PREFAULT_STACK_BYTES`].
pub const RT_STACK_BYTES: usize = 8 * 1024 * 1024;

/// Apply `cfg` to the calling thread. Returns one message per requested step that failed.
pub fn setup_current_thread(cfg: &RtConfig) -> Vec<String> {
    let mut failed = Vec::new();
    imp::apply(cfg, &mut failed);
    prefault_stack();
    failed
}

#[inline(never)]
fn prefault_stack() {
    let mut buf = [0u8; PREFAULT_STACK_BYTES];
    for i in (0..buf.len()).step_by(4096) {
        // SAFETY: in bounds; volatile so the touches aren't optimised away.
        unsafe { std::ptr::write_volatile(&mut buf[i], 1) };
    }
    std::hint::black_box(&buf);
}

#[cfg(target_os = "linux")]
mod imp {
    use super::RtConfig;
    use std::io;

    pub fn apply(cfg: &RtConfig, failed: &mut Vec<String>) {
        if cfg.lock_memory {
            // SAFETY: plain syscall.
            if unsafe { libc::mlockall(libc::MCL_CURRENT | libc::MCL_FUTURE) } != 0 {
                failed.push(format!("mlockall: {}", io::Error::last_os_error()));
            }
        }
        if let Some(cpu) = cfg.cpu {
            // SAFETY: zeroed cpu_set_t is empty; CPU_SET bounds-checks against the set size.
            let rc = unsafe {
                let mut set: libc::cpu_set_t = std::mem::zeroed();
                libc::CPU_SET(cpu, &mut set);
                libc::sched_setaffinity(0, size_of::<libc::cpu_set_t>(), &set)
            };
            if rc != 0 {
                failed.push(format!("pin to cpu {cpu}: {}", io::Error::last_os_error()));
            }
        }
        if let Some(priority) = cfg.priority {
            // SAFETY: a zeroed sched_param is valid (musl has extra fields); then set the priority.
            let rc = unsafe {
                let mut param: libc::sched_param = std::mem::zeroed();
                param.sched_priority = priority;
                libc::pthread_setschedparam(libc::pthread_self(), libc::SCHED_FIFO, &param)
            };
            if rc != 0 {
                failed.push(format!(
                    "SCHED_FIFO {priority}: {}",
                    io::Error::from_raw_os_error(rc)
                ));
            }
        }
    }
}

#[cfg(not(target_os = "linux"))]
mod imp {
    use super::RtConfig;

    pub fn apply(cfg: &RtConfig, failed: &mut Vec<String>) {
        let os = std::env::consts::OS;
        if cfg.lock_memory {
            failed.push(format!("mlockall: not supported on {os}"));
        }
        if cfg.cpu.is_some() {
            failed.push(format!("cpu pinning: not supported on {os}"));
        }
        if cfg.priority.is_some() {
            failed.push(format!("SCHED_FIFO: not supported on {os}"));
        }
    }
}
