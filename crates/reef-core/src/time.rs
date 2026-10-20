use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

/// Milliseconds since the Unix epoch.
pub fn now_ms() -> i64 {
    let d = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
    d.as_millis() as i64
}

/// GitHub-style timestamp: `2026-09-27T10:15:30Z` (seconds precision, always UTC).
pub fn iso(ms: i64) -> String {
    let secs = ms.div_euclid(1000);
    OffsetDateTime::from_unix_timestamp(secs)
        .ok()
        .and_then(|t| t.format(&Rfc3339).ok())
        .unwrap_or_else(|| "1970-01-01T00:00:00Z".to_string())
}

pub fn iso_opt(ms: Option<i64>) -> Option<String> {
    ms.map(iso)
}

pub fn parse_iso(s: &str) -> Option<i64> {
    OffsetDateTime::parse(s, &Rfc3339).ok().map(|t| (t.unix_timestamp_nanos() / 1_000_000) as i64)
}

#[cfg(test)]
mod tests {
    #[test]
    fn formats_like_github() {
        assert_eq!(super::iso(1_700_000_000_123), "2023-11-14T22:13:20Z");
        assert_eq!(super::parse_iso("2023-11-14T22:13:20Z"), Some(1_700_000_000_000));
    }
}
