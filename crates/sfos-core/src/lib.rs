//! Parsing and normalization for Sophos Firewall (SFOS) syslog messages.
//!
//! SFOS forwards logs as space separated `key=value` pairs, with values either
//! bare or double quoted. A single device can emit ~37 log types (firewall,
//! content filtering, IPS, anti-virus, VPN, authentication, ...), all sharing
//! that envelope but with different optional fields. This crate turns a raw
//! syslog line into a typed [`Event`] plus an `attrs` map holding everything the
//! normalized schema does not model explicitly.

pub mod model;
pub mod parse;

pub use model::{Action, CategoryGroup, Event};
pub use parse::{parse, parse_with_offset, ParseError};

/// The `timezone=` field Sophos writes is an abbreviation from the device's
/// configured timezone. Mapped to an offset in minutes east of UTC.
///
/// Abbreviations are ambiguous in general (IST is India *or* Ireland, CST is
/// China *or* US Central). Sophos appliances default to UTC; when an
/// abbreviation is not in this table the caller supplied default offset is used
/// instead, so deployments in less common zones stay correct.
pub fn timezone_offset_minutes(tz: &str) -> Option<i32> {
    let tz = tz.trim();
    let tz = tz.strip_prefix("UTC").unwrap_or(tz);
    if let Some(rest) = tz.strip_prefix('+') {
        return parse_hhmm(rest);
    }
    if let Some(rest) = tz.strip_prefix('-') {
        return parse_hhmm(rest).map(|m| -m);
    }
    let minutes = match tz.to_ascii_uppercase().as_str() {
        "" | "Z" | "UTC" | "GMT" | "UT" | "UCT" | "ZULU" => 0,
        "CET" | "WEST" | "BST" => 60,
        "CEST" | "EEST" | "MEST" | "WEDT" => 120,
        "EET" | "SAST" | "CAT" => 120,
        "MSK" => 180,
        "IST" => 330,
        "SGT" | "HKT" | "AWST" | "PHT" | "WITA" => 480,
        "JST" | "KST" | "WIT" => 540,
        "ACST" => 570,
        "AEST" | "CHST" => 600,
        "ACDT" => 630,
        "AEDT" => 660,
        "NZST" => 720,
        "NZDT" => 780,
        "AST" => -240,
        "EST" | "CDT" => -300,
        "EDT" => -240,
        "CST" => -360,
        "MST" => -420,
        "MDT" => -360,
        "PST" => -480,
        "PDT" => -420,
        "AKST" => -540,
        "AKDT" => -480,
        "HST" => -600,
        _ => return None,
    };
    Some(minutes)
}

fn parse_hhmm(s: &str) -> Option<i32> {
    let (h, m) = s.split_once(':')?;
    let h: i32 = h.trim().parse().ok()?;
    let m: i32 = m.trim().parse().ok()?;
    if !(0..=14).contains(&h) || !(0..60).contains(&m) {
        return None;
    }
    Some(h * 60 + m)
}

#[cfg(test)]
mod tests {
    use super::timezone_offset_minutes;

    #[test]
    fn parses_timezone_abbreviations() {
        assert_eq!(timezone_offset_minutes("UTC"), Some(0));
        assert_eq!(timezone_offset_minutes(""), Some(0));
        assert_eq!(timezone_offset_minutes("UTC+05:30"), Some(330));
        assert_eq!(timezone_offset_minutes("+02:00"), Some(120));
        assert_eq!(timezone_offset_minutes("-07:00"), Some(-420));
        assert_eq!(timezone_offset_minutes("AEST"), Some(600));
        assert_eq!(timezone_offset_minutes("Mars/Olympus"), None);
    }
}
