//! Cycle timing statistics. Fixed size and `Pod`, so the RT thread can record without allocating and
//! publish them on a topic.

use bytemuck::{Pod, Zeroable};
use iceoryx2_bb_derive_macros::ZeroCopySend;
use iceoryx2_bb_elementary_traits::zero_copy_send::ZeroCopySend;

/// Log2 buckets in microseconds: bucket 0 is < 1 µs, bucket k is [2^(k-1), 2^k) µs, the last is open.
pub const BUCKETS: usize = 24;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod, Zeroable, ZeroCopySend)]
#[type_name("elbows.Histogram/1")]
pub struct Histogram {
    pub counts: [u64; BUCKETS],
    pub max_ns: u64,
}

impl Histogram {
    pub fn record(&mut self, ns: u64) {
        let us = ns / 1000;
        let bucket = if us == 0 {
            0
        } else {
            (64 - us.leading_zeros()) as usize
        };
        self.counts[bucket.min(BUCKETS - 1)] += 1;
        self.max_ns = self.max_ns.max(ns);
    }

    pub fn total(&self) -> u64 {
        self.counts.iter().sum()
    }

    /// Upper bound, in µs, of the bucket holding quantile `q` (0..=1). 0 if empty.
    pub fn quantile_us(&self, q: f64) -> u64 {
        let target = (self.total() as f64 * q).ceil().max(1.0) as u64;
        let mut seen = 0;
        for (k, &c) in self.counts.iter().enumerate() {
            seen += c;
            if seen >= target {
                return 1 << k;
            }
        }
        0
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod, Zeroable, ZeroCopySend)]
#[type_name("elbows.CycleStats/1")]
pub struct CycleStats {
    pub cycles: u64,
    /// Cycles skipped because a previous one overran.
    pub missed: u64,
    /// Wake-up latency: deadline to the thread running.
    pub latency: Histogram,
    /// Time spent doing the cycle's work.
    pub exec: Histogram,
}

impl CycleStats {
    pub fn record(&mut self, latency_ns: u64, exec_ns: u64, missed: u64) {
        self.cycles += 1;
        self.missed += missed;
        self.latency.record(latency_ns);
        self.exec.record(exec_ns);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buckets_and_quantiles() {
        let mut h = Histogram::default();
        for ns in [500, 1_500, 3_000, 3_500, 100_000] {
            h.record(ns);
        }
        assert_eq!(h.counts[0], 1); // < 1 µs
        assert_eq!(h.counts[1], 1); // [1, 2) µs
        assert_eq!(h.counts[2], 2); // [2, 4) µs
        assert_eq!(h.max_ns, 100_000);
        assert_eq!(h.quantile_us(0.5), 4);
        assert_eq!(h.quantile_us(1.0), 128);
        assert_eq!(Histogram::default().quantile_us(0.99), 0);
    }
}
