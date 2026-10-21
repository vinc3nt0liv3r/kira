use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Coarse classification used for dashboard aggregation. Every event lands in
/// exactly one group, which keeps the SQL simple and the numbers additive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CategoryGroup {
    Web,
    Threat,
    Firewall,
    Auth,
    Vpn,
    System,
    Other,
}

impl CategoryGroup {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Web => "web",
            Self::Threat => "threat",
            Self::Firewall => "firewall",
            Self::Auth => "auth",
            Self::Vpn => "vpn",
            Self::System => "system",
            Self::Other => "other",
        }
    }

    /// Classify by SFOS `log_type` / `log_component`. `log_type` is the primary
    /// signal; component is used to catch web requests logged under generic
    /// types (e.g. SSL/TLS inspection logs).
    pub fn classify(log_type: &str, log_component: &str) -> Self {
        let t = log_type.trim().to_ascii_lowercase();
        let c = log_component.trim().to_ascii_lowercase();
        let all = format!("{t} {c}");
        let has = |needles: &[&str]| needles.iter().any(|n| all.contains(n));

        // Order matters: an IPS event is a threat even though it may mention a
        // firewall rule, and a firewall authentication event is auth, not traffic.
        if t.contains("content filtering") || t.contains("web filter") || t.contains("web proxy") {
            return Self::Web;
        }
        if has(&[
            "intrusion",
            "anti-virus",
            "antivirus",
            "anti-spam",
            "atp",
            "sandstorm",
            "zero-day",
            "web application firewall",
            "security heartbeat",
        ]) {
            return Self::Threat;
        }
        if has(&["authentication", "my account", "logon", "login"]) {
            return Self::Auth;
        }
        if t.contains("vpn") || t.contains("ipsec") || c.contains("vpn") {
            return Self::Vpn;
        }
        if t.contains("firewall")
            || t.contains("icmp")
            || t.contains("dos")
            || t.contains("mac filter")
        {
            return Self::Firewall;
        }
        if t.contains("ssl") || t.contains("tls") || c.contains("http") {
            return Self::Web;
        }
        if t.contains("system")
            || t.contains("gui")
            || t.contains("dhcp")
            || t.contains("dns")
            || t.contains("heartbeat")
            || t.contains("wireless")
        {
            return Self::System;
        }
        Self::Other
    }
}

/// Normalized allow/block state derived from `status` / `log_subtype`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    Allowed,
    Blocked,
    Warned,
}

impl Action {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Allowed => "allowed",
            Self::Blocked => "blocked",
            Self::Warned => "warned",
        }
    }

    pub fn from_status(status: &str, subtype: &str) -> Option<Self> {
        let s = format!("{status} {subtype}").to_ascii_lowercase();
        let has = |needles: &[&str]| needles.iter().any(|n| s.contains(n));
        if has(&["block", "denied", "deny", "drop", "reject", "discard"]) {
            Some(Self::Blocked)
        } else if has(&["allowed", "allow", "pass", "accept"]) {
            Some(Self::Allowed)
        } else if has(&["warn"]) {
            Some(Self::Warned)
        } else {
            None
        }
    }
}

/// One normalized syslog event.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Event {
    pub ts: DateTime<Utc>,
    pub device_id: Option<String>,
    pub device_name: Option<String>,
    pub log_id: Option<String>,
    pub log_type: String,
    pub log_component: Option<String>,
    pub log_subtype: Option<String>,
    /// `Start` / `Stop` for connections that SFOS logs twice. Dashboard
    /// aggregates count only the final record to avoid doubling connections.
    pub con_event: Option<String>,
    pub status: Option<String>,
    pub priority: Option<String>,
    pub category_group: CategoryGroup,
    pub action: Option<Action>,

    pub src_ip: Option<String>,
    pub dst_ip: Option<String>,
    pub src_port: Option<i32>,
    pub dst_port: Option<i32>,
    pub protocol: Option<String>,
    pub src_country: Option<String>,
    pub dst_country: Option<String>,
    pub src_zone: Option<String>,
    pub dst_zone: Option<String>,

    pub user_name: Option<String>,
    pub user_group: Option<String>,

    pub domain: Option<String>,
    pub url: Option<String>,
    pub http_method: Option<String>,
    pub http_status: Option<i32>,
    pub mime_type: Option<String>,
    pub referer: Option<String>,
    pub user_agent: Option<String>,
    pub category: Option<String>,
    pub category_type: Option<String>,
    pub application: Option<String>,
    pub application_category: Option<String>,
    pub application_risk: Option<i32>,

    pub fw_rule_id: Option<i64>,
    pub fw_rule_name: Option<String>,
    pub policy_type: Option<String>,

    pub sent_bytes: Option<i64>,
    pub recv_bytes: Option<i64>,

    pub threat_name: Option<String>,
    pub filename: Option<String>,
    pub reason: Option<String>,
    pub message: Option<String>,

    pub vpn_user: Option<String>,
    pub vpn_tunnel: Option<String>,

    /// Everything else on the line, keyed by the original SFOS field name.
    pub attrs: BTreeMap<String, String>,
}

impl Default for CategoryGroup {
    fn default() -> Self {
        Self::Other
    }
}
