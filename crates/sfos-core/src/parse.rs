use std::borrow::Cow;
use std::collections::BTreeMap;
use std::net::IpAddr;

use chrono::{DateTime, FixedOffset, NaiveDate, NaiveTime, TimeZone, Utc};
use thiserror::Error;

use crate::model::{Action, CategoryGroup, Event};
use crate::timezone_offset_minutes;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ParseError {
    #[error("empty syslog line")]
    Empty,
    #[error("no key=value pairs found")]
    NoFields,
    #[error("missing or invalid event timestamp")]
    Timestamp,
    #[error("not a supported Sophos Firewall log: {0}")]
    NotSophos(String),
}

/// Parse a Sophos Firewall syslog line, treating unknown timezone abbreviations
/// as UTC. Use [`parse_with_offset`] when the appliance runs in a zone Sophos
/// abbreviates ambiguously.
pub fn parse(line: &str) -> Result<Event, ParseError> {
    parse_with_offset(line, 0)
}

pub fn parse_with_offset(line: &str, default_offset_minutes: i32) -> Result<Event, ParseError> {
    let payload = strip_syslog_header(line);
    if payload.trim().is_empty() {
        return Err(ParseError::Empty);
    }
    let fields = split_fields(payload);
    if fields.is_empty() {
        return Err(ParseError::NoFields);
    }

    let lookup = |names: &[&str]| -> Option<&str> {
        names
            .iter()
            .find_map(|name| fields.get(*name).map(|v| v.as_ref()))
            .map(str::trim)
            .filter(|v| !v.is_empty() && *v != "-")
    };

    let log_type = lookup(&["log_type"]).unwrap_or_default().to_string();
    if log_type.is_empty() && lookup(&["log_id"]).is_none() {
        return Err(ParseError::NotSophos(truncate(payload, 120)));
    }
    if let Some(device) = lookup(&["device"]) {
        // Sophos UTM (SG) uses a different, positional format that is not modeled.
        if !device.eq_ignore_ascii_case("SFW") && !device.eq_ignore_ascii_case("SFOS") {
            return Err(ParseError::NotSophos(format!("device={device}")));
        }
    }

    let ts = parse_timestamp(&fields, lookup(&["timezone"]), default_offset_minutes)?;
    let log_component = lookup(&["log_component"]).unwrap_or_default().to_string();
    let status = lookup(&["status"]).map(str::to_string);
    let log_subtype = lookup(&["log_subtype"]).map(str::to_string);
    let action = Action::from_status(
        status.as_deref().unwrap_or_default(),
        log_subtype.as_deref().unwrap_or_default(),
    );

    let mut attrs: BTreeMap<String, String> = fields
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    for known in KNOWN_FIELDS {
        attrs.remove(*known);
    }

    Ok(Event {
        ts,
        // Older SFOS builds log `device_id`; 21+ logs the same value as
        // `device_serial_id`. Keep one canonical field for the dashboard.
        device_id: lookup(&["device_id", "device_serial_id"]).map(str::to_string),
        device_name: lookup(&["device_name"]).map(str::to_string),
        log_id: lookup(&["log_id"]).map(str::to_string),
        log_type,
        log_component: (!log_component.is_empty()).then_some(log_component),
        log_subtype,
        con_event: lookup(&["con_event"]).map(str::to_string),
        status,
        priority: lookup(&["priority", "severity"]).map(str::to_string),
        category_group: CategoryGroup::classify(
            lookup(&["log_type"]).unwrap_or_default(),
            lookup(&["log_component"]).unwrap_or_default(),
        ),
        action,

        src_ip: ip(lookup(&["src_ip", "srcip"])),
        dst_ip: ip(lookup(&["dst_ip", "dstip"])),
        src_port: int(lookup(&["src_port", "srcport"])),
        dst_port: int(lookup(&["dst_port", "dstport"])),
        protocol: lookup(&["protocol"]).map(str::to_string),
        src_country: lookup(&["src_country_code", "src_country", "srccountry"])
            .map(str::to_string),
        dst_country: lookup(&["dst_country_code", "dst_country", "dstcountry"])
            .map(str::to_string),
        src_zone: lookup(&["srczone", "src_zone"]).map(str::to_string),
        dst_zone: lookup(&["dstzone", "dst_zone"]).map(str::to_string),

        user_name: lookup(&["user_name", "username", "user"]).map(str::to_string),
        user_group: lookup(&["user_gp", "user_group", "usergroup", "group"]).map(str::to_string),

        domain: lookup(&["domain"]).map(str::to_string),
        url: lookup(&["url"]).map(str::to_string),
        http_method: lookup(&["http_method", "method"]).map(str::to_string),
        http_status: int(lookup(&["http_status", "status_code"])),
        mime_type: lookup(&["mime_type", "content_type"]).map(str::to_string),
        referer: lookup(&["referer"]).map(str::to_string),
        user_agent: lookup(&["user_agent", "http_user_agent"]).map(str::to_string),
        category: lookup(&["category", "http_category"]).map(str::to_string),
        application: lookup(&["application", "app_name"]).map(str::to_string),
        application_category: lookup(&["application_category", "app_category"])
            .map(str::to_string),
        application_risk: int(lookup(&["application_risk", "app_risk"])),

        fw_rule_id: int(lookup(&["fw_rule_id", "fwrule", "rule_id"])).map(i64::from),
        fw_rule_name: lookup(&["fw_rule_name", "rule_name"]).map(str::to_string),
        policy_type: lookup(&["policy_type"]).map(str::to_string),

        sent_bytes: int64(lookup(&["sent_bytes", "sentbyte", "bytes_sent", "tx_bytes"])),
        recv_bytes: int64(lookup(&["recv_bytes", "rcvdbyte", "bytes_received", "rx_bytes"])),

        threat_name: lookup(&["threat_name", "threatname"]).map(str::to_string),
        filename: lookup(&["filename"]).map(str::to_string),
        reason: lookup(&["reason"]).map(str::to_string),
        message: lookup(&["message"]).map(str::to_string),

        vpn_user: lookup(&["vpn_user", "tunnel_username"]).map(str::to_string),
        vpn_tunnel: lookup(&["vpn_tunnel", "tunnel_name"]).map(str::to_string),

        attrs,
    })
}

const KNOWN_FIELDS: &[&str] = &[
    "device", "date", "time", "timezone", "device_id", "device_serial_id", "device_name",
    "log_id", "log_type",
    "log_component", "log_subtype", "con_event", "status", "priority", "severity", "src_ip",
    "srcip", "dst_ip", "dstip",
    "src_port", "srcport", "dst_port", "dstport", "protocol", "src_country_code", "srccountry",
    "dst_country_code", "dstcountry", "srczone", "src_zone", "dstzone", "dst_zone", "user_name",
    "username", "user", "user_gp", "user_group", "usergroup", "group", "domain", "url",
    "http_method", "method", "http_status", "status_code", "mime_type", "content_type", "referer",
    "user_agent", "http_user_agent", "category", "http_category", "application",
    "application_category",
    "application_risk",
    "fw_rule_id", "fwrule", "rule_id", "fw_rule_name", "rule_name", "policy_type", "sent_bytes",
    "sentbyte", "tx_bytes", "recv_bytes", "rcvdbyte", "rx_bytes", "threat_name", "threatname",
    "filename", "reason", "message", "vpn_user", "tunnel_username", "vpn_tunnel", "tunnel_name",
    "timestamp", "app_name", "app_category", "app_risk", "bytes_sent", "bytes_received",
    "src_country", "dst_country",
];

/// Drop an optional RFC3164/RFC5424 header and return the Sophos payload.
/// SFOS usually sends the bare `device="SFW" date=...` payload, but syslog
/// relays commonly prepend priority, timestamp and hostname.
fn strip_syslog_header(line: &str) -> &str {
    let line = line.trim_start_matches('\u{feff}').trim();
    let start = if line.starts_with('<') {
        // <134> - priority value, always short.
        line.find('>').filter(|i| *i <= 5).map(|i| i + 1).unwrap_or(0)
    } else {
        0
    };
    let rest = &line[start..];

    // SFOS 21+ payloads start directly with `device_name=` / `timestamp=`;
    // only strip when the message really is wrapped in a syslog header.
    const PAYLOAD_PREFIXES: [&str; 6] = [
        "device=",
        "device_name=",
        "timestamp=",
        "date=",
        "log_id=",
        "log_type=",
    ];
    if PAYLOAD_PREFIXES.iter().any(|prefix| rest.starts_with(prefix)) {
        return rest.trim();
    }

    let mut best: Option<usize> = None;
    for marker in PAYLOAD_PREFIXES {
        let mut from = 0;
        while let Some(found) = rest[from..].find(marker) {
            let idx = from + found;
            let at_boundary = rest[..idx]
                .chars()
                .next_back()
                .is_none_or(|c| c.is_whitespace());
            if at_boundary {
                best = Some(best.map_or(idx, |b| b.min(idx)));
                break;
            }
            from = idx + marker.len();
        }
    }
    best.map(|i| rest[i..].trim()).unwrap_or(rest.trim())
}

/// Split `key=value` pairs. Values may be quoted, may contain spaces and `=`
/// (URLs, base64), and may be bare with spaces (`ether_type=IPv4 (0x0800)`).
/// A new field is only recognized as `key=` at a whitespace boundary.
fn split_fields(payload: &str) -> BTreeMap<&str, Cow<'_, str>> {
    let bytes = payload.as_bytes();
    let mut starts: Vec<(usize, usize, usize)> = Vec::new(); // key_start, value_start, key_end

    let is_key_char = |b: u8| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.');
    let mut i = 0;
    while i < bytes.len() {
        let at_boundary = i == 0 || bytes[i - 1].is_ascii_whitespace();
        if at_boundary && (bytes[i].is_ascii_alphabetic() || bytes[i] == b'_') {
            let mut j = i;
            while j < bytes.len() && is_key_char(bytes[j]) {
                j += 1;
            }
            if j < bytes.len() && bytes[j] == b'=' && j - i <= 64 {
                starts.push((i, j + 1, j));
            }
            i = j.max(i + 1);
            continue;
        }
        i += 1;
    }

    let mut fields = BTreeMap::new();
    for (n, (key_start, value_start, key_end)) in starts.iter().enumerate() {
        let end = starts
            .get(n + 1)
            .map(|(next_start, _, _)| *next_start)
            .unwrap_or(bytes.len());
        if end <= *value_start {
            continue;
        }
        let value = unquote(payload[*value_start..end].trim());
        let key = &payload[*key_start..*key_end];
        fields.entry(key).or_insert(value);
    }
    fields
}

fn unquote(raw: &str) -> Cow<'_, str> {
    let b = raw.as_bytes();
    if b.len() >= 2 && b[0] == b'"' && b[b.len() - 1] == b'"' {
        let inner = &raw[1..raw.len() - 1];
        if !inner.contains('\\') {
            return Cow::Borrowed(inner);
        }
        let mut out = String::with_capacity(inner.len());
        let mut chars = inner.chars();
        while let Some(c) = chars.next() {
            if c == '\\' {
                match chars.next() {
                    Some('n') => out.push('\n'),
                    Some('t') => out.push('\t'),
                    Some('r') => out.push('\r'),
                    Some(other) => out.push(other),
                    None => {}
                }
            } else {
                out.push(c);
            }
        }
        return Cow::Owned(out);
    }
    Cow::Borrowed(raw)
}

fn parse_timestamp(
    fields: &BTreeMap<&str, Cow<'_, str>>,
    timezone: Option<&str>,
    default_offset_minutes: i32,
) -> Result<DateTime<Utc>, ParseError> {
    if let Some(raw) = fields.get("timestamp") {
        if let Some(dt) = parse_iso8601(raw) {
            return Ok(dt);
        }
    }
    if let Some(raw) = fields.get("time") {
        if let Some(dt) = parse_iso8601(raw) {
            return Ok(dt);
        }
    }

    let date = fields.get("date").ok_or(ParseError::Timestamp)?;
    let date = NaiveDate::parse_from_str(date.trim(), "%Y-%m-%d")
        .or_else(|_| NaiveDate::parse_from_str(date.trim(), "%d/%m/%Y"))
        .map_err(|_| ParseError::Timestamp)?;
    let time_raw = fields.get("time").ok_or(ParseError::Timestamp)?;
    let time = NaiveTime::parse_from_str(time_raw.trim(), "%H:%M:%S%.f")
        .or_else(|_| NaiveTime::parse_from_str(time_raw.trim(), "%H:%M:%S"))
        .map_err(|_| ParseError::Timestamp)?;

    let offset = timezone
        .and_then(timezone_offset_minutes)
        .unwrap_or(default_offset_minutes);
    let offset = FixedOffset::east_opt(offset * 60).ok_or(ParseError::Timestamp)?;
    let local = offset
        .from_local_datetime(&date.and_time(time))
        .single()
        .ok_or(ParseError::Timestamp)?;
    Ok(local.with_timezone(&Utc))
}

/// SFOS 21 writes `timestamp="2026-10-03T15:45:28+0500"`; RFC3339 generally uses
/// a colon in the offset, and chrono's generic RFC3339 parser is strict about
/// it, so try the offset-without-colon form too.
fn parse_iso8601(raw: &str) -> Option<DateTime<Utc>> {
    let raw = raw.trim();
    for format in [
        "%Y-%m-%dT%H:%M:%S%z",
        "%Y-%m-%dT%H:%M:%S%.f%z",
        "%Y-%m-%d %H:%M:%S%z",
    ] {
        if let Ok(dt) = DateTime::parse_from_str(raw, format) {
            return Some(dt.with_timezone(&Utc));
        }
    }
    DateTime::parse_from_rfc3339(raw)
        .ok()
        .map(|dt| dt.with_timezone(&Utc))
}

fn int(value: Option<&str>) -> Option<i32> {
    value.and_then(|v| v.trim().parse::<i32>().ok())
}

fn int64(value: Option<&str>) -> Option<i64> {
    value.and_then(|v| {
        let v = v.trim();
        v.parse::<i64>()
            .ok()
            .or_else(|| v.parse::<f64>().ok().map(|f| f as i64))
    })
}

fn ip(value: Option<&str>) -> Option<String> {
    let value = value?;
    value
        .parse::<IpAddr>()
        .ok()
        .map(|ip| ip.to_string())
        .filter(|ip| ip != "0.0.0.0" && ip != "::")
}

fn truncate(s: &str, max: usize) -> String {
    s.chars().take(max).collect()
}
