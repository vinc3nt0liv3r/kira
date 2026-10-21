use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use serde_json::{json, Value};

#[derive(Default)]
pub struct Metrics {
    pub datagrams: AtomicU64,
    pub lines: AtomicU64,
    pub parsed: AtomicU64,
    pub rejected: AtomicU64,
    pub denied: AtomicU64,
    pub queued: AtomicU64,
    pub written: AtomicU64,
    pub db_errors: AtomicU64,
    pub dropped: AtomicU64,
    pub queue_high_water: AtomicU64,
}

impl Metrics {
    pub fn record_queue_depth(&self, depth: usize) {
        self.queue_high_water
            .fetch_max(depth as u64, Ordering::Relaxed);
    }

    pub fn snapshot(&self, queue_depth: usize, queue_capacity: usize) -> Value {
        self.record_queue_depth(queue_depth);
        let queue_utilization_percent = if queue_capacity == 0 {
            0.0
        } else {
            queue_depth as f64 * 100.0 / queue_capacity as f64
        };
        json!({
            "datagrams": self.datagrams.load(Ordering::Relaxed),
            "lines": self.lines.load(Ordering::Relaxed),
            "parsed": self.parsed.load(Ordering::Relaxed),
            "rejected": self.rejected.load(Ordering::Relaxed),
            "denied_sources": self.denied.load(Ordering::Relaxed),
            "queued": self.queued.load(Ordering::Relaxed),
            "written": self.written.load(Ordering::Relaxed),
            "db_errors": self.db_errors.load(Ordering::Relaxed),
            "dropped": self.dropped.load(Ordering::Relaxed),
            "queue_depth": queue_depth,
            "queue_capacity": queue_capacity,
            "queue_utilization_percent": queue_utilization_percent,
            "queue_high_water": self.queue_high_water.load(Ordering::Relaxed),
        })
    }
}

pub type SharedMetrics = Arc<Metrics>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exposes_queue_pressure_and_high_water() {
        let metrics = Metrics::default();
        metrics.record_queue_depth(12);
        let snapshot = metrics.snapshot(8, 20);

        assert_eq!(snapshot["queue_depth"], 8);
        assert_eq!(snapshot["queue_capacity"], 20);
        assert_eq!(snapshot["queue_utilization_percent"], 40.0);
        assert_eq!(snapshot["queue_high_water"], 12);
    }
}
