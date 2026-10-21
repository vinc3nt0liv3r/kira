use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::time::Duration;

use sfos_core::Event;
use sfos_store::Store;
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::listener::Incoming;
use crate::metrics::SharedMetrics;

/// Drain the queue into per-tenant batches and COPY them into Postgres.
/// Writes are retried a few times so a brief database blip does not silently
/// lose data; a longer outage drops the batch and increments `dropped`.
pub async fn run(
    store: Store,
    mut rx: mpsc::Receiver<Incoming>,
    batch_size: usize,
    batch_ms: u64,
    metrics: SharedMetrics,
) {
    let mut batch: HashMap<Uuid, Vec<Event>> = HashMap::new();
    let mut count = 0usize;
    let mut ticker = tokio::time::interval(Duration::from_millis(batch_ms));
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    loop {
        tokio::select! {
            maybe = rx.recv() => {
                match maybe {
                    Some(incoming) => {
                        batch.entry(incoming.tenant_id).or_default().push(incoming.event);
                        count += 1;
                        if count >= batch_size {
                            flush(&store, &mut batch, &metrics).await;
                            count = 0;
                        }
                    }
                    None => break,
                }
            }
            _ = ticker.tick() => {
                if count > 0 {
                    flush(&store, &mut batch, &metrics).await;
                    count = 0;
                }
            }
        }
    }
    if count > 0 {
        flush(&store, &mut batch, &metrics).await;
    }
}

async fn flush(store: &Store, batch: &mut HashMap<Uuid, Vec<Event>>, metrics: &SharedMetrics) {
    for (tenant_id, events) in batch.drain() {
        let mut attempt = 0u32;
        loop {
            match store.copy_events(tenant_id, &events).await {
                Ok(rows) => {
                    metrics.written.fetch_add(rows, Ordering::Relaxed);
                    break;
                }
                Err(err) => {
                    attempt += 1;
                    metrics.db_errors.fetch_add(1, Ordering::Relaxed);
                    if attempt >= 3 {
                        metrics
                            .dropped
                            .fetch_add(events.len() as u64, Ordering::Relaxed);
                        tracing::error!(
                            %err,
                            rows = events.len(),
                            "failed to write batch after 3 attempts; dropping"
                        );
                        break;
                    }
                    tracing::warn!(%err, attempt, "batch write failed, retrying");
                    tokio::time::sleep(Duration::from_millis(200 * attempt as u64)).await;
                }
            }
        }
    }
}
