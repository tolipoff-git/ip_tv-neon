use chrono::{DateTime, NaiveDateTime, Utc};
use std::fs::OpenOptions;
use std::io::Write;

pub fn main_log(msg: &str) {
    let dir = crate::consts::get_cache_dir();
    let _ = std::fs::create_dir_all(&dir);
    let log_path = dir.join("neon_iptv.log");
    let _ = OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path)
        .and_then(|mut f| writeln!(f, "[{}] {}", Utc::now().format("%H:%M:%S"), msg));
}

pub fn normalize(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_alphanumeric())
        .collect::<String>()
        .to_lowercase()
}

/// Strip terminal control characters from untrusted text before it reaches any
/// TTY/ratatui sink (CG8). Replacing every C0/C1 control char — including ESC
/// (0x1B) and BEL (0x07) — with a space neutralizes ANSI CSI / OSC sequences:
/// once the introducer ESC is gone the trailing bytes are inert printable text.
pub fn sanitize_terminal(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect()
}

/// Parse an XMLTV timestamp into a Unix epoch, or 0 if unrecognized.
///
/// Accepts, in order: `YYYYMMDDHHMMSS +ZZZZ` (with explicit offset),
/// `YYYYMMDDHHMMSS` and `YYYYMMDDHHMM` (minutes-only). A timestamp without an
/// explicit timezone offset is assumed to be naive UTC — XMLTV feeds without a
/// `+ZZZZ` suffix are treated as UTC wall-clock here.
pub fn parse_xml_time(s: &str) -> i64 {
    let s = s.trim();
    if let Ok(dt) = DateTime::parse_from_str(s, "%Y%m%d%H%M%S %z") {
        return dt.timestamp();
    }
    if let Ok(dt) = DateTime::parse_from_str(s, "%Y%m%d%H%M %z") {
        return dt.timestamp();
    }
    if let Ok(dt) = NaiveDateTime::parse_from_str(s, "%Y%m%d%H%M%S") {
        return dt.and_utc().timestamp();
    }
    if let Ok(dt) = NaiveDateTime::parse_from_str(s, "%Y%m%d%H%M") {
        return dt.and_utc().timestamp();
    }
    0
}

/// Mask credentials in a source URL for diagnostics output.
///
/// Tokens usually live in a long path segment (`/iptv/TOKEN/…`) or in the query
/// string; both are replaced so `ip_tv-neon diag` can be pasted into a bug
/// report without leaking access to the provider.
pub fn mask_url(raw: &str) -> String {
    let s = raw.trim();
    if s.is_empty() {
        return "(not set)".into();
    }
    if !s.contains("://") {
        return s.to_string(); // local path — no credentials
    }
    let (base, has_query) = match s.split_once('?') {
        Some((b, _)) => (b, true),
        None => (s, false),
    };
    let mut out = String::new();
    for (i, part) in base.split('/').enumerate() {
        if i > 0 {
            out.push('/');
        }
        if i < 3 || part.chars().count() <= 16 {
            out.push_str(part);
        } else {
            out.push_str("***");
        }
    }
    if has_query {
        out.push_str("?***");
    }
    out
}

/// Compact relative age ("just now", "12m ago", "3h ago", "5d ago").
pub fn human_age(now: i64, then: i64) -> String {
    if then <= 0 {
        return "never".into();
    }
    let secs = (now - then).max(0);
    match secs {
        0..=59 => "just now".into(),
        60..=3599 => format!("{}m ago", secs / 60),
        3600..=86_399 => format!("{}h ago", secs / 3600),
        _ => format!("{}d ago", secs / 86_400),
    }
}

#[cfg(test)]
mod tests {
    use super::{human_age, mask_url, parse_xml_time};
    use chrono::{DateTime, NaiveDateTime};

    #[test]
    fn parses_full_with_offset() {
        let want = DateTime::parse_from_str("20240115123000 +0300", "%Y%m%d%H%M%S %z")
            .unwrap()
            .timestamp();
        assert_eq!(parse_xml_time("20240115123000 +0300"), want);
    }

    #[test]
    fn parses_naive_seconds_as_utc() {
        let want = NaiveDateTime::parse_from_str("20240115123000", "%Y%m%d%H%M%S")
            .unwrap()
            .and_utc()
            .timestamp();
        assert_eq!(parse_xml_time("20240115123000"), want);
    }

    #[test]
    fn parses_minutes_only() {
        let want = NaiveDateTime::parse_from_str("202401151230", "%Y%m%d%H%M")
            .unwrap()
            .and_utc()
            .timestamp();
        assert_eq!(parse_xml_time("202401151230"), want);
        assert!(parse_xml_time("202401151230") > 0);
    }

    #[test]
    fn parses_minutes_only_with_offset() {
        assert!(parse_xml_time("202401151230 +0000") > 0);
    }

    #[test]
    fn garbage_returns_zero() {
        assert_eq!(parse_xml_time("not-a-time"), 0);
        assert_eq!(parse_xml_time(""), 0);
        assert_eq!(parse_xml_time("2024"), 0);
    }

    #[test]
    fn trims_whitespace() {
        assert!(parse_xml_time("  20240115123000  ") > 0);
    }

    #[test]
    fn masks_token_path_and_query() {
        assert_eq!(
            mask_url("http://provider.example/iptv/SUPERSECRETTOKEN123/204/index.m3u8?token=abc"),
            "http://provider.example/iptv/***/204/index.m3u8?***"
        );
        assert_eq!(
            mask_url("http://127.0.0.1:8095/neon.m3u"),
            "http://127.0.0.1:8095/neon.m3u"
        );
        assert_eq!(mask_url(""), "(not set)");
        assert_eq!(mask_url("/home/admin/list.m3u"), "/home/admin/list.m3u");
    }

    #[test]
    fn human_age_buckets() {
        let now = 1_000_000;
        assert_eq!(human_age(now, 0), "never");
        assert_eq!(human_age(now, now - 5), "just now");
        assert_eq!(human_age(now, now - 120), "2m ago");
        assert_eq!(human_age(now, now - 7200), "2h ago");
        assert_eq!(human_age(now, now - 2 * 86_400), "2d ago");
        assert_eq!(human_age(now, now + 100), "just now");
    }
}
