use chrono::{TimeZone, Utc};
use sfos_core::{parse, parse_with_offset, Action, CategoryGroup, ParseError};

/// Captured SFOS 21 XGS lines (anonymized) using `<pri>device_name=... timestamp="...+0500"`.
const SFOS21: &str = include_str!("fixtures/sfos21.log");

const FIREWALL: &str = r#"device="SFW" date=2025-04-14 time=10:30:00 timezone="UTC" device_name="XGS2100" device_id=C010012345678 log_id=010101600001 log_type="Firewall" log_component="Firewall Rule" log_subtype="Allowed" status="Allow" priority=Information duration=120 fw_rule_id=5 fw_rule_name="Allow LAN to WAN" fw_rule_section="user" policy_type=0 user_name="stefan" user_gp="" iap=0 appfilter_policy_id=0 application="" application_risk=0 application_category="" ether_type=IPv4 (0x0800) in_interface="Port2" src_mac=00:0c:29:ab:cd:ef src_ip=192.168.1.50 src_country_code=R1 dst_ip=8.8.8.8 dst_country_code=USA protocol="TCP" src_port=54321 dst_port=443 sent_pkts=15 recv_pkts=12 sent_bytes=2048 recv_bytes=4096 srczone="LAN" dstzone="WAN" connid="1234567890" message="" app_category="general-internet""#;

const WEB_BLOCKED: &str = r#"device="SFW" date=2025-04-14 time=10:32:00 timezone="UTC" device_name="XGS2100" device_id=C010012345678 log_id=031006209001 log_type="Content Filtering" log_component="HTTP" log_subtype="Denied" status="Denied" priority="Warning" fw_rule_id=3 fw_rule_name="LAN to WAN" user_name="alice" user_gp="Staff" src_ip=192.168.1.50 src_country_code=R1 dst_ip=142.250.185.78 protocol="TCP" src_port=51234 dst_port=443 domain="www.google.com" category="Search Engines" category_type="Unproductive" url="https://www.google.com/search?q=sophos+firewall&hl=en" http_method="GET" http_status=403 mime_type="text/html" sent_bytes=512 recv_bytes=0"#;

#[test]
fn parses_firewall_traffic() {
    let ev = parse(FIREWALL).unwrap();
    assert_eq!(ev.log_type, "Firewall");
    assert_eq!(ev.log_component.as_deref(), Some("Firewall Rule"));
    assert_eq!(ev.category_group, CategoryGroup::Firewall);
    assert_eq!(ev.action, Some(Action::Allowed));
    assert_eq!(ev.user_name.as_deref(), Some("stefan"));
    assert_eq!(ev.fw_rule_id, Some(5));
    assert_eq!(ev.fw_rule_name.as_deref(), Some("Allow LAN to WAN"));
    assert_eq!(ev.src_ip.as_deref(), Some("192.168.1.50"));
    assert_eq!(ev.dst_port, Some(443));
    assert_eq!(ev.sent_bytes, Some(2048));
    assert_eq!(ev.recv_bytes, Some(4096));
    assert_eq!(ev.src_zone.as_deref(), Some("LAN"));
    assert_eq!(ev.ts, Utc.with_ymd_and_hms(2025, 4, 14, 10, 30, 0).unwrap());
    // A bare value with a space must not swallow the next field.
    assert_eq!(ev.attrs.get("ether_type").map(String::as_str), Some("IPv4 (0x0800)"));
    assert_eq!(ev.attrs.get("connid").map(String::as_str), Some("1234567890"));
    // Fields modeled as columns are not duplicated into attrs.
    assert!(!ev.attrs.contains_key("src_ip"));
    assert!(!ev.attrs.contains_key("date"));
}

#[test]
fn parses_blocked_web_request_and_keeps_url_query() {
    let ev = parse(WEB_BLOCKED).unwrap();
    assert_eq!(ev.category_group, CategoryGroup::Web);
    assert_eq!(ev.action, Some(Action::Blocked));
    assert_eq!(ev.domain.as_deref(), Some("www.google.com"));
    assert_eq!(ev.category.as_deref(), Some("Search Engines"));
    assert_eq!(
        ev.url.as_deref(),
        Some("https://www.google.com/search?q=sophos+firewall&hl=en")
    );
    assert_eq!(ev.http_method.as_deref(), Some("GET"));
    assert_eq!(ev.http_status, Some(403));
    assert_eq!(ev.attrs.get("category_type").map(String::as_str), Some("Unproductive"));
}

#[test]
fn parses_threat_log_and_keeps_av_fields_in_attrs() {
    let line = r#"device="SFW" date=2025-04-14 time=10:32:00 timezone="UTC" device_name="XG310" device_id=C010012345678 log_id=054002616001 log_type="Anti-Virus" log_component="SMTP" log_subtype="Virus" status="" priority=Warning src_ip=192.168.1.100 dst_ip=10.0.0.5 protocol="TCP" src_port=25 dst_port=1025 message="" reason="" filename="malware.exe" threatname="EICAR-Test-File" quarantine="Yes" quarantine_reason="Infected""#;
    let ev = parse(line).unwrap();
    assert_eq!(ev.category_group, CategoryGroup::Threat);
    assert_eq!(ev.threat_name.as_deref(), Some("EICAR-Test-File"));
    assert_eq!(ev.filename.as_deref(), Some("malware.exe"));
    assert_eq!(ev.attrs.get("quarantine").map(String::as_str), Some("Yes"));
    assert_eq!(ev.action, None);
}

#[test]
fn classifies_vpn_and_authentication_events() {
    let vpn = r#"device="SFW" date=2025-04-14 time=11:00:00 timezone="UTC" device_name="XGS2100" device_id=C01 log_id=041303617601 log_type="VPN" log_component="SSL VPN" log_subtype="Connected" status="Success" priority="Information" user_name="bob" vpn_user="bob" tunnel_name="SSLVPNSite" src_ip=203.0.113.5 message="User bob connected""#;
    let ev = parse(vpn).unwrap();
    assert_eq!(ev.category_group, CategoryGroup::Vpn);
    assert_eq!(ev.vpn_user.as_deref(), Some("bob"));
    assert_eq!(ev.vpn_tunnel.as_deref(), Some("SSLVPNSite"));
    assert_eq!(ev.src_ip.as_deref(), Some("203.0.113.5"));

    let auth = r#"device="SFW" date=2025-04-14 time=10:31:00 timezone="UTC" device_name="XG310" device_id=C01 log_id=062009617704 log_type="Event" log_component="FirewallAuthentication" log_subtype="Authentication" status="" priority=Information user_name="admin" src_ip=10.0.0.1 message="User admin of group Open Group logged in successfully through Local authentication mechanism""#;
    let ev = parse(auth).unwrap();
    assert_eq!(ev.category_group, CategoryGroup::Auth);
    assert_eq!(ev.user_name.as_deref(), Some("admin"));
}

#[test]
fn strips_rfc3164_and_rfc5424_headers() {
    let rfc3164 = format!("<134>Apr 14 10:30:00 XGS2100 SFOS: {FIREWALL}");
    assert_eq!(parse(&rfc3164).unwrap().log_id.as_deref(), Some("010101600001"));

    let rfc5424 = format!("<134>1 2025-04-14T10:30:00.123Z xgs2100 sfos - - - {WEB_BLOCKED}");
    let ev = parse(&rfc5424).unwrap();
    assert_eq!(ev.category_group, CategoryGroup::Web);
    assert_eq!(ev.user_name.as_deref(), Some("alice"));
}

#[test]
fn applies_timezone_abbreviation_and_default_offset() {
    let aest = r#"device="SFW" date=2025-04-14 time=10:30:00 timezone="AEST" log_id=010101600001 log_type="Firewall" status="Allow""#;
    assert_eq!(
        parse(aest).unwrap().ts,
        Utc.with_ymd_and_hms(2025, 4, 14, 0, 30, 0).unwrap()
    );

    let unknown = r#"device="SFW" date=2025-04-14 time=10:30:00 timezone="XYZ" log_id=010101600001 log_type="Firewall""#;
    assert_eq!(
        parse_with_offset(unknown, -300).unwrap().ts,
        Utc.with_ymd_and_hms(2025, 4, 14, 15, 30, 0).unwrap()
    );
}

#[test]
fn unescapes_quoted_values() {
    let line = r#"device="SFW" date=2025-04-14 time=10:30:00 timezone="UTC" log_id=010101600001 log_type="Firewall" message="User \"bob\" said C:\\temp""#;
    let ev = parse(line).unwrap();
    assert_eq!(ev.message.as_deref(), Some("User \"bob\" said C:\\temp"));
}

#[test]
fn parses_sfos21_iso_timestamps_and_byte_fields() {
    let lines: Vec<&str> = SFOS21.lines().filter(|line| !line.trim().is_empty()).collect();
    assert_eq!(lines.len(), 7);
    for line in &lines {
        parse(line).expect("every captured SFOS 21 line must parse");
    }

    let start = parse(lines[0]).unwrap();
    assert_eq!(start.ts, Utc.with_ymd_and_hms(2026, 10, 3, 10, 45, 28).unwrap());
    assert_eq!(start.device_name.as_deref(), Some("X33010KMQVXTJ26"));
    // SFOS 21 logs the serial as device_serial_id; the dashboard needs it as device_id.
    assert_eq!(start.device_id.as_deref(), Some("X33010KMQVXTJ26"));
    assert!(!start.attrs.contains_key("device_serial_id"));
    assert_eq!(start.log_component.as_deref(), Some("Firewall Rule"));
    assert_eq!(start.con_event.as_deref(), Some("Start"));
    assert_eq!(start.priority.as_deref(), Some("Information"));
    assert_eq!(start.src_country.as_deref(), Some("R1"));
    assert_eq!(start.fw_rule_name.as_deref(), Some("UTM-Bronnze_Acc"));
    assert_eq!(start.sent_bytes, None);
    assert_eq!(start.category_group, CategoryGroup::Firewall);
    assert_eq!(start.action, Some(Action::Allowed));

    let stop = parse(lines[1]).unwrap();
    assert_eq!(stop.con_event.as_deref(), Some("Stop"));
    assert_eq!(stop.sent_bytes, Some(100));
    assert_eq!(stop.recv_bytes, Some(60));

    let web = lines
        .iter()
        .map(|line| parse(line).unwrap())
        .find(|event| event.category.is_some())
        .expect("a content filtering line");
    assert_eq!(web.category_group, CategoryGroup::Web);
    assert!(web.url.as_deref().unwrap_or_default().starts_with("http"));
    assert!(web.sent_bytes.is_some());
    assert_eq!(web.category.as_deref(), Some("Information Technology"));
    assert!(web.user_agent.is_some());
    assert!(!web.attrs.contains_key("http_user_agent"));

    let web_category = lines
        .iter()
        .map(|line| parse(line).unwrap())
        .find(|event| event.category.as_deref() == Some("Software Updates"))
        .expect("an SFOS 21 HTTP category");
    assert_eq!(web_category.category.as_deref(), Some("Software Updates"));
    assert!(!web_category.attrs.contains_key("http_category"));

    let app = lines
        .iter()
        .map(|line| parse(line).unwrap())
        .find(|event| event.application.is_some())
        .expect("an application control line");
    assert_eq!(app.application.as_deref(), Some("Windows Update"));
    assert_eq!(app.application_category.as_deref(), Some("Software Update"));
    assert_eq!(app.application_risk, Some(3));
    assert_eq!(app.action, Some(Action::Blocked));

    let spoof = lines
        .iter()
        .map(|line| parse(line).unwrap())
        .find(|event| event.log_component.as_deref() == Some("IP Spoof"))
        .expect("an IP spoof line");
    assert_eq!(spoof.category_group, CategoryGroup::Firewall);
    assert_eq!(spoof.action, Some(Action::Blocked));

    // The same payload through a relay that prepends an RFC3164 header.
    let wrapped = format!("<134>Oct  3 10:45:28 xgs SFOS: {}", lines[0]);
    let relayed = parse(&wrapped).unwrap();
    assert_eq!(relayed.con_event.as_deref(), Some("Start"));
    assert_eq!(relayed.ts, start.ts);
}

#[test]
fn rejects_non_sophos_and_incomplete_lines() {
    assert!(matches!(
        parse("this is just noise").unwrap_err(),
        ParseError::NoFields
    ));
    assert!(matches!(
        parse("foo=bar baz=qux").unwrap_err(),
        ParseError::NotSophos(_)
    ));
    assert!(matches!(parse("").unwrap_err(), ParseError::Empty));
    assert!(matches!(
        parse(r#"device="SFW" log_id=010101600001 log_type="Firewall""#).unwrap_err(),
        ParseError::Timestamp
    ));
    assert!(matches!(
        parse(r#"device="UTM" date=2025-04-14 time=10:30:00 log_id=1 log_type="Firewall""#)
            .unwrap_err(),
        ParseError::NotSophos(_)
    ));
}
