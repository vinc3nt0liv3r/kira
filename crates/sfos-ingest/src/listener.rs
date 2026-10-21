use std::net::SocketAddr;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use anyhow::{Context, Result};
use sfos_core::Event;
use tokio::io::AsyncReadExt;
use tokio::net::{TcpListener, TcpStream, UdpSocket};
use tokio::sync::{mpsc, Semaphore};
use uuid::Uuid;

use crate::metrics::SharedMetrics;
use crate::sources::SourceMap;

const MAX_LINE: usize = 64 * 1024;
const MAX_CONNECTIONS: usize = 256;

pub struct Incoming {
    pub tenant_id: Uuid,
    pub event: Event,
}

pub async fn run_udp(
    socket: UdpSocket,
    sources: Arc<SourceMap>,
    tx: mpsc::Sender<Incoming>,
    metrics: SharedMetrics,
    tz_offset_minutes: i32,
    keep_extended_attributes: bool,
) -> Result<()> {
    let mut buf = vec![0u8; 65_535];
    loop {
        let (len, peer) = match socket.recv_from(&mut buf).await {
            Ok(v) => v,
            Err(err) => {
                tracing::warn!(%err, "udp recv failed");
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                continue;
            }
        };
        metrics.datagrams.fetch_add(1, Ordering::Relaxed);
        let Some(tenant_id) = sources.resolve(peer.ip()) else {
            metrics.denied.fetch_add(1, Ordering::Relaxed);
            continue;
        };
        let text = String::from_utf8_lossy(&buf[..len]);
        for line in text.trim_end_matches(['\0', '\n', '\r']).lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            metrics.lines.fetch_add(1, Ordering::Relaxed);
            match sfos_core::parse_with_offset(line, tz_offset_minutes) {
                Ok(event) => {
                    let mut event = event;
                    if !keep_extended_attributes {
                        retain_compact_attributes(&mut event);
                    }
                    metrics.parsed.fetch_add(1, Ordering::Relaxed);
                    if tx.send(Incoming { tenant_id, event }).await.is_err() {
                        return Ok(());
                    }
                    metrics.queued.fetch_add(1, Ordering::Relaxed);
                    metrics.record_queue_depth(tx.max_capacity() - tx.capacity());
                }
                Err(err) => reject(&metrics, line, &err.to_string()),
            }
        }
    }
}

pub async fn run_tcp(
    listener: TcpListener,
    sources: Arc<SourceMap>,
    tx: mpsc::Sender<Incoming>,
    metrics: SharedMetrics,
    tz_offset_minutes: i32,
    keep_extended_attributes: bool,
) -> Result<()> {
    let semaphore = Arc::new(Semaphore::new(MAX_CONNECTIONS));
    loop {
        let (stream, peer) = listener.accept().await.context("tcp accept failed")?;
        let Some(tenant_id) = sources.resolve(peer.ip()) else {
            metrics.denied.fetch_add(1, Ordering::Relaxed);
            drop(stream);
            continue;
        };
        let permit = match semaphore.clone().acquire_owned().await {
            Ok(p) => p,
            Err(_) => return Ok(()),
        };
        let tx = tx.clone();
        let metrics = metrics.clone();
        tokio::spawn(async move {
            if let Err(err) = handle_tcp(
                stream,
                peer,
                tenant_id,
                tx,
                metrics.clone(),
                tz_offset_minutes,
                keep_extended_attributes,
            )
            .await
            {
                tracing::debug!(%peer, %err, "tcp connection ended with error");
            }
            drop(permit);
        });
    }
}

async fn handle_tcp(
    mut stream: TcpStream,
    peer: SocketAddr,
    tenant_id: Uuid,
    tx: mpsc::Sender<Incoming>,
    metrics: SharedMetrics,
    tz_offset_minutes: i32,
    keep_extended_attributes: bool,
) -> Result<()> {
    let mut chunk = [0u8; 8192];
    let mut pending: Vec<u8> = Vec::with_capacity(4096);
    let mut discarding = false;
    loop {
        let n = stream.read(&mut chunk).await?;
        if n == 0 {
            if !pending.is_empty() {
                dispatch_line(
                    &pending,
                    tenant_id,
                    &tx,
                    &metrics,
                    tz_offset_minutes,
                    keep_extended_attributes,
                )
                .await?;
            }
            break;
        }
        let mut start = 0;
        for (i, byte) in chunk[..n].iter().enumerate() {
            if *byte != b'\n' {
                continue;
            }
            if discarding {
                discarding = false;
                pending.clear();
                start = i + 1;
                continue;
            }
            pending.extend_from_slice(&chunk[start..i]);
            let line = std::mem::take(&mut pending);
            if dispatch_line(
                &line,
                tenant_id,
                &tx,
                &metrics,
                tz_offset_minutes,
                keep_extended_attributes,
            )
            .await
            .is_err()
            {
                return Ok(());
            }
            start = i + 1;
            pending = Vec::with_capacity(4096);
        }
        if start < n && !discarding {
            pending.extend_from_slice(&chunk[start..n]);
            if pending.len() > MAX_LINE {
                metrics.rejected.fetch_add(1, Ordering::Relaxed);
                tracing::warn!(%peer, "tcp syslog line exceeded 64 KiB, discarding");
                pending.clear();
                discarding = true;
            }
        }
    }
    Ok(())
}

async fn dispatch_line(
    line: &[u8],
    tenant_id: Uuid,
    tx: &mpsc::Sender<Incoming>,
    metrics: &SharedMetrics,
    tz_offset_minutes: i32,
    keep_extended_attributes: bool,
) -> Result<()> {
    let text = String::from_utf8_lossy(line);
    let text = text.trim();
    if text.is_empty() {
        return Ok(());
    }
    metrics.lines.fetch_add(1, Ordering::Relaxed);
    match sfos_core::parse_with_offset(text, tz_offset_minutes) {
        Ok(mut event) => {
            if !keep_extended_attributes {
                retain_compact_attributes(&mut event);
            }
            metrics.parsed.fetch_add(1, Ordering::Relaxed);
            tx.send(Incoming { tenant_id, event })
                .await
                .map_err(|_| anyhow::anyhow!("writer stopped"))?;
            metrics.queued.fetch_add(1, Ordering::Relaxed);
            metrics.record_queue_depth(tx.max_capacity() - tx.capacity());
        }
        Err(err) => reject(metrics, text, &err.to_string()),
    }
    Ok(())
}

fn retain_compact_attributes(event: &mut Event) {
    event
        .attrs
        .retain(|key, _| matches!(key.as_str(), "device_model" | "firmware_version"));
}

/// Rejections are expected (other vendors on the same port, truncated packets).
/// Log the first few and then every 500th to avoid flooding the log.
fn reject(metrics: &SharedMetrics, line: &str, reason: &str) {
    let count = metrics.rejected.fetch_add(1, Ordering::Relaxed) + 1;
    if count <= 5 || count % 500 == 0 {
        let sample: String = line.chars().take(160).collect();
        tracing::warn!(
            rejected = count,
            reason,
            sample,
            "dropped unparseable syslog line"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compact_attributes_preserve_device_metadata_only() {
        let mut event = Event::default();
        event.attrs.insert("device_model".into(), "XGS".into());
        event.attrs.insert("firmware_version".into(), "20.0".into());
        event.attrs.insert("mac".into(), "00:11:22:33:44:55".into());

        retain_compact_attributes(&mut event);

        assert_eq!(event.attrs.len(), 2);
        assert_eq!(
            event.attrs.get("device_model").map(String::as_str),
            Some("XGS")
        );
        assert!(!event.attrs.contains_key("mac"));
    }
}
