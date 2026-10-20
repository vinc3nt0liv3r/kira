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
}

impl Metrics {
    pub fn snapshot(&self) -> Value {
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
        })
    }
}

pub type SharedMetrics = Arc<Metrics>;
