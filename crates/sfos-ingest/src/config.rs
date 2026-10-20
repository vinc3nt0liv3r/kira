use std::net::{IpAddr, SocketAddr};
use std::str::FromStr;

use anyhow::{Context, Result};
use ipnet::IpNet;

#[derive(Debug, Clone)]
pub struct Config {
    pub database_url: String,
    pub database_password: Option<String>,
    pub db_pool_size: usize,
    pub udp_addr: SocketAddr,
    pub tcp_addr: SocketAddr,
    pub health_addr: SocketAddr,
    pub tenant_slug: String,
    pub tenant_name: String,
    /// Optional `CIDR=slug` map routing logs from a firewall/relay to a tenant.
    pub tenant_map: Vec<(IpNet, String)>,
    /// When non-empty, only these source IPs/CIDRs may send logs.
    pub allowed_sources: Vec<IpNet>,
    pub default_tz_offset_minutes: i32,
    pub retention_hours: i64,
    pub batch_size: usize,
    pub batch_ms: u64,
    pub partitions_ahead_hours: i64,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        let database_url = std::env::var("DATABASE_URL")
            .context("DATABASE_URL is required (e.g. postgres://user:pass@host/db)")?;
        let default_tz_offset_minutes = env_parse("DEFAULT_TZ_OFFSET_MINUTES", 0)?;
        anyhow::ensure!(
            (-720..=840).contains(&default_tz_offset_minutes),
            "DEFAULT_TZ_OFFSET_MINUTES must be between -720 and 840"
        );

        Ok(Self {
            database_url,
            database_password: std::env::var("DATABASE_PASSWORD").ok(),
            db_pool_size: env_parse("DB_POOL_SIZE", 8)?,
            udp_addr: env_parse("SYSLOG_UDP_ADDR", "0.0.0.0:5514".parse()?)?,
            tcp_addr: env_parse("SYSLOG_TCP_ADDR", "0.0.0.0:5514".parse()?)?,
            health_addr: env_parse("HEALTH_ADDR", "0.0.0.0:8081".parse()?)?,
            tenant_slug: std::env::var("TENANT_SLUG").unwrap_or_else(|_| "default".into()),
            tenant_name: std::env::var("TENANT_NAME").unwrap_or_else(|_| "Default site".into()),
            tenant_map: parse_tenant_map(std::env::var("TENANT_MAP").ok().as_deref())?,
            allowed_sources: parse_networks(std::env::var("ALLOWED_SOURCES").ok().as_deref())?,
            default_tz_offset_minutes,
            retention_hours: env_parse("RETENTION_HOURS", 6)?.clamp(1, 168),
            batch_size: env_parse::<usize>("BATCH_SIZE", 5_000)?.clamp(1, 10_000),
            batch_ms: env_parse::<u64>("BATCH_MS", 2_000)?.clamp(50, 60_000),
            partitions_ahead_hours: env_parse("PARTITIONS_AHEAD_HOURS", 48)?.clamp(1, 168),
        })
    }
}

fn env_parse<T>(key: &str, default: T) -> Result<T>
where
    T: FromStr,
    T::Err: std::fmt::Display,
{
    match std::env::var(key) {
        Ok(raw) if !raw.trim().is_empty() => raw
            .trim()
            .parse::<T>()
            .map_err(|e| anyhow::anyhow!("invalid {key}={raw:?}: {e}")),
        _ => Ok(default),
    }
}

fn parse_networks(raw: Option<&str>) -> Result<Vec<IpNet>> {
    let Some(raw) = raw else {
        return Ok(Vec::new());
    };
    raw.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| parse_net(s).with_context(|| format!("invalid CIDR {s:?}")))
        .collect()
}

/// Accept both CIDR (`10.0.0.0/8`) and a single address (`203.0.113.5`).
fn parse_net(raw: &str) -> Result<IpNet> {
    if let Ok(net) = raw.parse::<IpNet>() {
        return Ok(net);
    }
    raw.parse::<IpAddr>()
        .map(IpNet::from)
        .map_err(|e| anyhow::anyhow!("{e}"))
}

fn parse_tenant_map(raw: Option<&str>) -> Result<Vec<(IpNet, String)>> {
    let Some(raw) = raw else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    for entry in raw.split(',').map(str::trim).filter(|s| !s.is_empty()) {
        let (cidr, slug) = entry
            .split_once('=')
            .with_context(|| format!("TENANT_MAP entry {entry:?} must look like 192.0.2.0/24=acme"))?;
        let slug = slug.trim();
        anyhow::ensure!(!slug.is_empty(), "TENANT_MAP entry {entry:?} has an empty slug");
        out.push((
            parse_net(cidr.trim())
                .with_context(|| format!("invalid CIDR in TENANT_MAP entry {entry:?}"))?,
            slug.to_string(),
        ));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_tenant_map_and_networks() {
        let map = parse_tenant_map(Some("192.0.2.0/24=acme, 198.51.100.7=beta")).unwrap();
        assert_eq!(map.len(), 2);
        assert_eq!(map[1].1, "beta");
        assert!(parse_tenant_map(Some("nope")).is_err());

        let nets = parse_networks(Some("10.0.0.0/8,203.0.113.5")).unwrap();
        assert_eq!(nets.len(), 2);
        assert!(parse_networks(Some("10.0.0.0/99")).is_err());
    }
}
