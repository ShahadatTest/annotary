//! Timestamps: RFC 3339 UTC with millisecond precision, e.g. `2026-08-18T09:41:02.123Z`.
//! Millis keep same-second mutations ordered (agents push fast); the CLI trims to seconds for
//! humans where the prototype shows seconds.

use jiff::Timestamp;

/// Current time as the canonical stored string.
pub fn now() -> String {
    format(Timestamp::now())
}

/// Format a timestamp the canonical way (UTC, millisecond precision, `Z`).
pub fn format(ts: Timestamp) -> String {
    ts.strftime("%Y-%m-%dT%H:%M:%S%.3fZ").to_string()
}

/// Parse a stored timestamp (also accepts second precision and offsets).
pub fn parse(s: &str) -> Option<Timestamp> {
    s.parse().ok()
}

/// Trim a stored timestamp to seconds (`2026-08-18T09:41:02Z`) — the form shown in `--json`
/// and written into Sidecars. Unparsable input is returned as is.
pub fn to_seconds(s: &str) -> String {
    match parse(s) {
        Some(t) => t.strftime("%Y-%m-%dT%H:%M:%SZ").to_string(),
        None => s.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_with_millis() {
        let s = now();
        assert!(s.ends_with('Z'));
        assert_eq!(s.len(), "2026-08-18T09:41:02.123Z".len());
        let ts = parse(&s).unwrap();
        assert_eq!(format(ts), s);
        assert_eq!(to_seconds("2026-08-18T09:41:02.123Z"), "2026-08-18T09:41:02Z");
        assert_eq!(to_seconds("junk"), "junk");
    }
}
