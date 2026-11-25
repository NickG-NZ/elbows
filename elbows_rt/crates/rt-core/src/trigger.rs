//! What starts each RT cycle. The loop only sees [`Tick`]s, so the source can change freely.
//!
//! - [`ClockTrigger`]: fixed period on `CLOCK_MONOTONIC` (the default).
//! - Later, a device trigger: block on an fd such as an INS data-ready GPIO line, with a timeout of
//!   about 1.5 periods that falls back to the clock and counts the miss.

use std::time::Duration;

use crate::time::{monotonic_ns, sleep_until_ns};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Tick {
    /// Cycle number. Counts skipped cycles too, so `index * period` tracks elapsed time.
    pub index: u64,
    /// When this cycle was due.
    pub deadline_ns: u64,
    /// When the thread actually woke. Use this as "now" for the whole cycle.
    pub now_ns: u64,
    /// Whole cycles skipped since the previous tick because we were late (an overrun).
    pub missed: u64,
}

impl Tick {
    /// Wake-up latency: how late after its deadline this cycle started.
    pub fn latency_ns(&self) -> u64 {
        self.now_ns.saturating_sub(self.deadline_ns)
    }
}

pub trait Trigger: Send {
    /// Block until the next cycle is due.
    fn wait(&mut self) -> Tick;
    /// Nominal cycle period.
    fn period_ns(&self) -> u64;
}

/// Fixed-rate cycles on absolute deadlines (`next += period`), so timing error never accumulates.
/// If a cycle overruns by whole periods, those cycles are skipped and reported in [`Tick::missed`]
/// rather than run back to back.
pub struct ClockTrigger {
    period_ns: u64,
    next_ns: u64,
    index: u64,
}

impl ClockTrigger {
    pub fn new(period: Duration) -> Self {
        let period_ns = period.as_nanos().max(1) as u64;
        Self {
            period_ns,
            next_ns: monotonic_ns() + period_ns,
            index: 0,
        }
    }

    pub fn from_hz(hz: f64) -> Self {
        assert!(
            hz > 0.0 && hz.is_finite(),
            "rate must be positive, got {hz}"
        );
        Self::new(Duration::from_secs_f64(1.0 / hz))
    }
}

impl Trigger for ClockTrigger {
    fn wait(&mut self) -> Tick {
        sleep_until_ns(self.next_ns);
        let now_ns = monotonic_ns();
        let missed = now_ns.saturating_sub(self.next_ns) / self.period_ns;
        let deadline_ns = self.next_ns + missed * self.period_ns;
        self.index += missed;
        let tick = Tick {
            index: self.index,
            deadline_ns,
            now_ns,
            missed,
        };
        self.index += 1;
        self.next_ns = deadline_ns + self.period_ns;
        tick
    }

    fn period_ns(&self) -> u64 {
        self.period_ns
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_rate() {
        let mut t = ClockTrigger::new(Duration::from_millis(2));
        let first = t.wait();
        let mut last = first;
        for _ in 0..20 {
            last = t.wait();
        }
        // Deadlines stay on the grid even if a loaded machine makes us miss some.
        assert!(last.index - first.index >= 20);
        assert_eq!(
            last.deadline_ns - first.deadline_ns,
            (last.index - first.index) * 2_000_000
        );
        assert!(last.now_ns >= last.deadline_ns);
    }

    #[test]
    fn skips_and_reports_missed_cycles() {
        let mut t = ClockTrigger::new(Duration::from_millis(2));
        let a = t.wait();
        std::thread::sleep(Duration::from_millis(9)); // overrun by ~4 periods
        let b = t.wait();
        assert!(b.missed >= 3, "missed {}", b.missed);
        assert_eq!(b.index, a.index + 1 + b.missed);
        assert_eq!(
            (b.deadline_ns - a.deadline_ns) / 2_000_000,
            b.index - a.index
        );
    }
}
