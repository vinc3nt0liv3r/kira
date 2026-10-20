use sfos_core::Event;
use uuid::Uuid;

/// Column order must match the encoder below and the COPY statement.
pub const COLUMNS: &[&str] = &[
    "tenant_id",
    "ts",
    "device_id",
    "device_name",
    "log_id",
    "log_type",
    "log_component",
    "log_subtype",
    "con_event",
    "status",
    "priority",
    "category_group",
    "action",
    "src_ip",
    "dst_ip",
    "src_port",
    "dst_port",
    "protocol",
    "src_country",
    "dst_country",
    "src_zone",
    "dst_zone",
    "user_name",
    "user_group",
    "domain",
    "url",
    "http_method",
    "http_status",
    "mime_type",
    "referer",
    "user_agent",
    "category",
    "application",
    "application_category",
    "application_risk",
    "fw_rule_id",
    "fw_rule_name",
    "policy_type",
    "sent_bytes",
    "recv_bytes",
    "threat_name",
    "filename",
    "reason",
    "message",
    "vpn_user",
    "vpn_tunnel",
    "attrs",
];

pub fn copy_statement() -> String {
    copy_statement_for("events")
}

pub fn copy_statement_for(table: &str) -> String {
    format!(
        "copy {table} ({}) from stdin with (format text)",
        COLUMNS.join(", ")
    )
}

/// Encode one event as a text-format COPY row. COPY uses `\N` for NULL and
/// backslash escapes for tabs, newlines and backslashes.
pub fn encode_row(tenant_id: Uuid, ev: &Event, out: &mut String) {
    let row_start = out.len();
    let mut first = true;
    raw(out, &mut first, &tenant_id.to_string());
    raw(out, &mut first, &ev.ts.format("%Y-%m-%d %H:%M:%S%.6f%:z").to_string());

    text(out, &mut first, ev.device_id.as_deref());
    text(out, &mut first, ev.device_name.as_deref());
    text(out, &mut first, ev.log_id.as_deref());
    text(out, &mut first, Some(&ev.log_type));
    text(out, &mut first, ev.log_component.as_deref());
    text(out, &mut first, ev.log_subtype.as_deref());
    text(out, &mut first, ev.con_event.as_deref());
    text(out, &mut first, ev.status.as_deref());
    text(out, &mut first, ev.priority.as_deref());
    raw(out, &mut first, ev.category_group.as_str());
    text(out, &mut first, ev.action.map(|a| a.as_str()));

    text(out, &mut first, ev.src_ip.as_deref());
    text(out, &mut first, ev.dst_ip.as_deref());
    number(out, &mut first, ev.src_port);
    number(out, &mut first, ev.dst_port);
    text(out, &mut first, ev.protocol.as_deref());
    text(out, &mut first, ev.src_country.as_deref());
    text(out, &mut first, ev.dst_country.as_deref());
    text(out, &mut first, ev.src_zone.as_deref());
    text(out, &mut first, ev.dst_zone.as_deref());

    text(out, &mut first, ev.user_name.as_deref());
    text(out, &mut first, ev.user_group.as_deref());

    text(out, &mut first, ev.domain.as_deref());
    text(out, &mut first, ev.url.as_deref());
    text(out, &mut first, ev.http_method.as_deref());
    number(out, &mut first, ev.http_status);
    text(out, &mut first, ev.mime_type.as_deref());
    text(out, &mut first, ev.referer.as_deref());
    text(out, &mut first, ev.user_agent.as_deref());
    text(out, &mut first, ev.category.as_deref());
    text(out, &mut first, ev.application.as_deref());
    text(out, &mut first, ev.application_category.as_deref());
    number(out, &mut first, ev.application_risk);

    int64(out, &mut first, ev.fw_rule_id);
    text(out, &mut first, ev.fw_rule_name.as_deref());
    text(out, &mut first, ev.policy_type.as_deref());

    int64(out, &mut first, ev.sent_bytes);
    int64(out, &mut first, ev.recv_bytes);

    text(out, &mut first, ev.threat_name.as_deref());
    text(out, &mut first, ev.filename.as_deref());
    text(out, &mut first, ev.reason.as_deref());
    text(out, &mut first, ev.message.as_deref());
    text(out, &mut first, ev.vpn_user.as_deref());
    text(out, &mut first, ev.vpn_tunnel.as_deref());

    let attrs = serde_json::to_string(&ev.attrs).unwrap_or_else(|_| "{}".to_string());
    raw(out, &mut first, &attrs);
    out.push('\n');

    debug_assert_eq!(
        out[row_start..].trim_end_matches('\n').split('\t').count(),
        COLUMNS.len(),
        "COPY row arity must match COLUMNS"
    );
}

fn begin(out: &mut String, first: &mut bool) {
    if *first {
        *first = false;
    } else {
        out.push('\t');
    }
}

fn text(out: &mut String, first: &mut bool, value: Option<&str>) {
    begin(out, first);
    match value {
        Some(v) => escape(out, v),
        None => out.push_str("\\N"),
    }
}

fn raw(out: &mut String, first: &mut bool, value: &str) {
    begin(out, first);
    escape(out, value);
}

fn number(out: &mut String, first: &mut bool, value: Option<i32>) {
    match value {
        Some(v) => raw(out, first, &v.to_string()),
        None => {
            begin(out, first);
            out.push_str("\\N");
        }
    }
}

fn int64(out: &mut String, first: &mut bool, value: Option<i64>) {
    match value {
        Some(v) => raw(out, first, &v.to_string()),
        None => {
            begin(out, first);
            out.push_str("\\N");
        }
    }
}

fn escape(out: &mut String, value: &str) {
    for ch in value.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\0' => {}
            other => out.push(other),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LINE: &str = r#"device="SFW" date=2025-04-14 time=10:32:00 timezone="UTC" device_name="XGS2100" device_id=C01 log_id=031006209001 log_type="Content Filtering" log_component="HTTP" log_subtype="Denied" status="Denied" user_name="alice" src_ip=192.168.1.50 dst_ip=142.250.185.78 src_port=51234 dst_port=443 protocol="TCP" domain="www.google.com" category="Search Engines" url="https://www.google.com/search?q=a+b" http_status=403 sent_bytes=512 recv_bytes=0 message="line one"#;

    #[test]
    fn encodes_every_column() {
        let ev = sfos_core::parse(LINE).unwrap();
        let id = Uuid::nil();
        let mut out = String::new();
        encode_row(id, &ev, &mut out);

        let line = out.trim_end_matches('\n');
        let fields: Vec<&str> = line.split('\t').collect();
        assert_eq!(fields.len(), COLUMNS.len(), "columns: {COLUMNS:?}");
        assert_eq!(fields[0], "00000000-0000-0000-0000-000000000000");
        assert!(fields[1].starts_with("2025-04-14 10:32:00"));
        assert!(fields[1].ends_with("+00:00"));
        assert_eq!(fields[COLUMNS.iter().position(|c| *c == "domain").unwrap()], "www.google.com");
        assert_eq!(fields[COLUMNS.iter().position(|c| *c == "action").unwrap()], "blocked");
        assert_eq!(fields[COLUMNS.iter().position(|c| *c == "sent_bytes").unwrap()], "512");
    }

    #[test]
    fn escapes_control_characters_and_nulls() {
        let mut ev = sfos_core::parse(LINE).unwrap();
        ev.message = Some("a\tb\nc\\d".into());
        ev.threat_name = None;
        let mut out = String::new();
        encode_row(Uuid::nil(), &ev, &mut out);
        let line = out.trim_end_matches('\n');
        let fields: Vec<&str> = line.split('\t').collect();
        let message = fields[COLUMNS.iter().position(|c| *c == "message").unwrap()];
        let threat = fields[COLUMNS.iter().position(|c| *c == "threat_name").unwrap()];
        assert_eq!(message, "a\\tb\\nc\\\\d");
        assert_eq!(threat, "\\N");
    }
}
