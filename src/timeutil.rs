//! Myanmar Standard Time (UTC+6:30) helpers.
//!
//! The whole system is standardized on Asia/Yangon wall-clock time, exactly like
//! the original Node app (`process.env.TZ = 'Asia/Yangon'`). Datetimes are stored
//! in SQLite as local Yangon strings `YYYY-MM-DD HH:MM:SS`.

use chrono::{Duration, FixedOffset, NaiveDateTime, Utc};

pub const YANGON_OFFSET_SECS: i32 = 6 * 3600 + 30 * 60; // +06:30
pub const DT_FMT: &str = "%Y-%m-%d %H:%M:%S";
pub const DISPLAY_FMT: &str = "%Y-%m-%d %H:%M";

pub fn yangon_offset() -> FixedOffset {
    FixedOffset::east_opt(YANGON_OFFSET_SECS).expect("valid offset")
}

/// Current wall-clock time in Myanmar.
pub fn now_myanmar() -> NaiveDateTime {
    Utc::now().with_timezone(&yangon_offset()).naive_local()
}

/// Current Myanmar time formatted for DB storage.
pub fn now_str() -> String {
    now_myanmar().format(DT_FMT).to_string()
}

/// Default time stored for a match entered by hand (no kickoff time chosen).
pub const MATCH_DEFAULT_TIME: &str = "12:00:00";

/// The "matchday" (business date) a moment belongs to.
///
/// Uses a noon-to-noon window that is labelled by the day it STARTS, so an
/// evening match and the games that spill into the next morning share the same
/// matchday:  `business_date = DATE(dt - 12h)`.
/// e.g.  11th 20:00 -> 11;  12th 02:00 -> 11;  12th 13:00 -> 12.
pub fn business_date_of(dt: NaiveDateTime) -> String {
    (dt - Duration::hours(12)).format("%Y-%m-%d").to_string()
}

/// Today's matchday, labelled by the day the noon-to-noon window starts.
pub fn business_today() -> String {
    business_date_of(now_myanmar())
}

/// Matchday `days` days before today (used as a retention cutoff).
pub fn business_today_minus_days(days: i64) -> String {
    (now_myanmar() - Duration::hours(12) - Duration::days(days))
        .format("%Y-%m-%d")
        .to_string()
}

/// Parse a stored datetime string into a NaiveDateTime, if possible.
pub fn parse_dt(s: &str) -> Option<NaiveDateTime> {
    NaiveDateTime::parse_from_str(s.trim(), DT_FMT)
        .ok()
        .or_else(|| NaiveDateTime::parse_from_str(s.trim(), "%Y-%m-%dT%H:%M:%S").ok())
}

/// Format a stored datetime for display. Falls back to the raw string.
pub fn display_dt(s: &str) -> String {
    parse_dt(s)
        .map(|d| d.format(DISPLAY_FMT).to_string())
        .unwrap_or_else(|| s.to_string())
}

/// Format a number with thousands separators, keeping up to 2 decimals when needed.
pub fn money(v: f64) -> String {
    let neg = v < 0.0;
    let a = v.abs();
    let int_part = a.trunc() as i64;
    let frac = a - a.trunc();
    let int_str = group_digits(int_part);
    let s = if frac < 1e-9 {
        int_str
    } else {
        let mut f = format!("{:.2}", frac);
        f.remove(0); // drop leading '0'
        while f.ends_with('0') {
            f.pop();
        }
        format!("{}{}", int_str, f)
    };
    if neg {
        format!("-{}", s)
    } else {
        s
    }
}

fn group_digits(n: i64) -> String {
    let s = n.to_string();
    let bytes = s.as_bytes();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    let len = bytes.len();
    for (i, b) in bytes.iter().enumerate() {
        if i > 0 && (len - i) % 3 == 0 {
            out.push(',');
        }
        out.push(*b as char);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn business_date_groups_next_morning_with_previous_evening() {
        let d = |s: &str| business_date_of(parse_dt(s).unwrap());
        assert_eq!(d("2026-10-11 20:00:00"), "2026-10-11");
        assert_eq!(d("2026-10-12 02:00:00"), "2026-10-11");
        assert_eq!(d("2026-10-11 13:00:00"), "2026-10-11");
        assert_eq!(d("2026-10-12 13:00:00"), "2026-10-12");
        assert_eq!(d("2026-10-11 09:00:00"), "2026-10-10");
    }

    #[test]
    fn money_formats_with_commas() {
        assert_eq!(money(100000.0), "100,000");
        assert_eq!(money(1000.0), "1,000");
        assert_eq!(money(0.0), "0");
        assert_eq!(money(1234567.0), "1,234,567");
        assert_eq!(money(-5000.0), "-5,000");
        assert_eq!(money(100000.5), "100,000.5");
        assert_eq!(money(100000.95), "100,000.95");
    }
}
