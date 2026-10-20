use std::net::IpAddr;

use ipnet::IpNet;
use uuid::Uuid;

/// Routes an incoming syslog packet to a tenant. Multi-tenant deployments put
/// one collector in front of several firewalls (or relays) and distinguish them
/// by source address; single-tenant deployments use the default tenant.
pub struct SourceMap {
    default_tenant: Uuid,
    entries: Vec<(IpNet, Uuid)>,
    allowed: Vec<IpNet>,
}

impl SourceMap {
    pub fn new(default_tenant: Uuid, entries: Vec<(IpNet, Uuid)>, allowed: Vec<IpNet>) -> Self {
        Self {
            default_tenant,
            entries,
            allowed,
        }
    }

    pub fn resolve(&self, ip: IpAddr) -> Option<Uuid> {
        if !self.allowed.is_empty() && !self.allowed.iter().any(|net| net.contains(&ip)) {
            return None;
        }
        let best = self
            .entries
            .iter()
            .filter(|(net, _)| net.contains(&ip))
            .max_by_key(|(net, _)| net.prefix_len());
        Some(best.map(|(_, id)| *id).unwrap_or(self.default_tenant))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn longest_prefix_wins_and_allowed_sources_are_enforced() {
        let default = Uuid::new_v4();
        let acme = Uuid::new_v4();
        let beta = Uuid::new_v4();
        let map = SourceMap::new(
            default,
            vec![
                ("10.0.0.0/8".parse().unwrap(), acme),
                ("10.1.0.0/16".parse().unwrap(), beta),
            ],
            vec!["10.0.0.0/8".parse().unwrap()],
        );
        assert_eq!(map.resolve("10.2.3.4".parse().unwrap()), Some(acme));
        assert_eq!(map.resolve("10.1.2.3".parse().unwrap()), Some(beta));
        assert_eq!(map.resolve("192.168.1.1".parse().unwrap()), None);
    }
}
