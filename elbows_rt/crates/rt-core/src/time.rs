//! `CLOCK_MONOTONIC`: the one timebase for the robot. Shared by every process on the host, never
//! steps. Wall-clock time is never used for control.

/// Nanoseconds on `CLOCK_MONOTONIC`.
pub fn monotonic_ns() -> u64 {
    let mut ts = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: `ts` is a valid out-pointer; CLOCK_MONOTONIC always exists.
    unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts) };
    ts.tv_sec as u64 * 1_000_000_000 + ts.tv_nsec as u64
}

/// Sleep until `CLOCK_MONOTONIC` reaches `t_ns`. Returns at once if it already has.
pub fn sleep_until_ns(t_ns: u64) {
    #[cfg(target_os = "linux")]
    {
        let ts = libc::timespec {
            tv_sec: (t_ns / 1_000_000_000) as _,
            tv_nsec: (t_ns % 1_000_000_000) as _,
        };
        // An absolute deadline, so retrying after a signal can't drift.
        // SAFETY: `ts` is valid; a null remainder is allowed with TIMER_ABSTIME.
        while unsafe {
            libc::clock_nanosleep(
                libc::CLOCK_MONOTONIC,
                libc::TIMER_ABSTIME,
                &ts,
                std::ptr::null_mut(),
            )
        } == libc::EINTR
        {}
    }
    #[cfg(not(target_os = "linux"))]
    {
        // Dev machines (macOS) have no clock_nanosleep; relative sleep is close enough there.
        let now = monotonic_ns();
        if t_ns > now {
            std::thread::sleep(std::time::Duration::from_nanos(t_ns - now));
        }
    }
}
