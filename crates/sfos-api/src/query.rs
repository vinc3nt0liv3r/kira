use chrono::{DateTime, Duration, Utc};

#[derive(Debug, Clone, serde::Deserialize)]
pub struct RangeQuery {
    pub tenant: Option<String>,
    /// RFC3339, epoch seconds, or a relative window such as `24h`, `7d`, `90m`.
    pub from: Option<String>,
    pub to: Option<String>,
    /// Bucket width in seconds. Optional; chosen from the range when omitted.
    pub interval: Option<i64>,
}

#[derive(Debug, Clone, Copy)]
pub struct TimeRange {
    pub from: DateTime<Utc>,
    pub to: DateTime<Utc>,
    pub bucket_secs: i64,
}

impl RangeQuery {
    pub fn resolve(&self, now: DateTime<Utc>) -> Result<TimeRange, String> {
        let to = match self.to.as_deref() {
            Some(raw) => parse_bound(raw, now)?,
            None => now,
        };
        let from = match self.from.as_deref() {
            Some(raw) => parse_bound(raw, now)?,
            None => to - Duration::hours(24),
        };
        if from >= to {
            return Err("`from` must be earlier than `to`".into());
        }
        if to - from > Duration::days(366) {
            return Err("time range is limited to 366 days".into());
        }
        if from.timestamp() < 0 {
            return Err("`from` must be after 1970-01-01".into());
        }
        let bucket_secs = match self.interval {
            Some(secs) if !(5..=86_400).contains(&secs) => {
                return Err("`interval` must be between 5 and 86400 seconds".into())
            }
            Some(secs) => secs,
            None => default_bucket(to - from),
        };
        Ok(TimeRange {
            from,
            to,
            bucket_secs,
        })
    }
}

/// Keep chart payloads around 100-300 points regardless of range.
pub fn default_bucket(range: Duration) -> i64 {
    let secs = range.num_seconds().max(1);
    match secs {
        s if s <= 3_600 => 60,
        s if s <= 21_600 => 300,
        s if s <= 172_800 => 900,
        s if s <= 691_200 => 3_600,
        _ => 86_400,
    }
}

fn parse_bound(raw: &str, now: DateTime<Utc>) -> Result<DateTime<Utc>, String> {
    let raw = raw.trim();
    if raw.is_empty() || raw.eq_ignore_ascii_case("now") {
        return Ok(now);
    }
    if let Ok(epoch) = raw.parse::<i64>() {
        return DateTime::from_timestamp(epoch, 0)
            .ok_or_else(|| format!("epoch timestamp out of range: {raw}"));
    }
    if let Ok(dt) = DateTime::parse_from_rfc3339(raw) {
        return Ok(dt.with_timezone(&Utc));
    }
    let (number, unit) = raw.split_at(raw.len().saturating_sub(1));
    let amount: i64 = number
        .parse()
        .map_err(|_| format!("invalid time `{raw}` (use RFC3339, epoch seconds, or e.g. 24h)"))?;
    let duration = match unit {
        "m" => Duration::minutes(amount),
        "h" => Duration::hours(amount),
        "d" => Duration::days(amount),
        "w" => Duration::weeks(amount),
        _ => return Err(format!("invalid time unit in `{raw}` (use m, h, d or w)")),
    };
    Ok(now - duration)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 4, 14, 12, 0, 0).unwrap()
    }

    #[test]
    fn resolves_relative_and_absolute_ranges() {
        let q = RangeQuery {
            tenant: None,
            from: Some("24h".into()),
            to: None,
            interval: None,
        };
        let r = q.resolve(now()).unwrap();
        assert_eq!(r.from, now() - Duration::hours(24));
        assert_eq!(r.bucket_secs, 900);

        let q = RangeQuery {
            tenant: None,
            from: Some("2025-04-14T00:00:00Z".into()),
            to: Some("2025-04-14T06:00:00Z".into()),
            interval: Some(300),
        };
        let r = q.resolve(now()).unwrap();
        assert_eq!(r.bucket_secs, 300);
        assert_eq!(r.to.timestamp() - r.from.timestamp(), 21_600);
    }

    #[test]
    fn rejects_invalid_ranges() {
        let q = RangeQuery {
            tenant: None,
            from: Some("1d".into()),
            to: Some("2d".into()),
            interval: None,
        };
        assert!(q.resolve(now()).is_err());

        let q = RangeQuery {
            tenant: None,
            from: Some("yesterday".into()),
            to: None,
            interval: None,
        };
        assert!(q.resolve(now()).is_err());

        let q = RangeQuery {
            tenant: None,
            from: None,
            to: None,
            interval: Some(1),
        };
        assert!(q.resolve(now()).is_err());
    }

    #[test]
    fn picks_coarser_buckets_for_longer_ranges() {
        assert_eq!(default_bucket(Duration::hours(1)), 60);
        assert_eq!(default_bucket(Duration::hours(6)), 300);
        assert_eq!(default_bucket(Duration::days(2)), 900);
        assert_eq!(default_bucket(Duration::days(7)), 3_600);
        assert_eq!(default_bucket(Duration::days(30)), 86_400);
    }
}
