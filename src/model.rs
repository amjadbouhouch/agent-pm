//! Values shared by every command group: status categories, priority, `name=value` pairs,
//! dates and date-times.

use clap::ValueEnum;

use crate::error::{Result, err};

/// The group a project's custom status belongs to. Statuses are rows (`cmd::status`);
/// what a status means to `list`, milestone progress and the shortcuts comes from this.
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum Category {
    Todo,
    Active,
    Done,
    Cancelled,
}

impl Category {
    pub fn as_str(self) -> &'static str {
        match self {
            Category::Todo => "todo",
            Category::Active => "active",
            Category::Done => "done",
            Category::Cancelled => "cancelled",
        }
    }

    /// Tasks in done and cancelled statuses are hidden from `list` unless `--all`.
    pub fn is_closed(self) -> bool {
        matches!(self, Category::Done | Category::Cancelled)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum Priority {
    None,
    Low,
    Medium,
    High,
    Urgent,
}

impl Priority {
    pub fn as_str(self) -> &'static str {
        match self {
            Priority::None => "none",
            Priority::Low => "low",
            Priority::Medium => "medium",
            Priority::High => "high",
            Priority::Urgent => "urgent",
        }
    }
}

/// clap value parser for `name=value` (used by `--set` and `--where`).
pub fn parse_pair(s: &str) -> std::result::Result<(String, String), String> {
    match s.split_once('=') {
        Some((k, v)) if !k.trim().is_empty() => Ok((k.trim().to_string(), v.to_string())),
        _ => Err(format!("expected name=value, got `{s}`")),
    }
}

/// Validate a `YYYY-MM-DD` date (calendar-checked, no time zone).
pub fn check_date(s: &str) -> Result<()> {
    let bad = || err(format!("invalid date `{s}`, expected YYYY-MM-DD"));
    let parts: Vec<&str> = s.split('-').collect();
    if parts.len() != 3 || parts[0].len() != 4 || parts[1].len() != 2 || parts[2].len() != 2 {
        return Err(bad());
    }
    let n = |p: &str| p.parse::<u32>().map_err(|_| bad());
    let (y, m, d) = (n(parts[0])?, n(parts[1])?, n(parts[2])?);
    let leap = (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
    let days = match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return Err(bad()),
    };
    if d == 0 || d > days {
        return Err(bad());
    }
    Ok(())
}

/// A "when" value: a date, or a date-time normalized to UTC.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum When {
    /// `YYYY-MM-DD`, as given.
    Date(String),
    /// `YYYY-MM-DDTHH:MM:SSZ`, in UTC.
    DateTime(String),
}

impl When {
    /// The stored form. As text, a date sorts before every date-time on the same day.
    pub fn as_str(&self) -> &str {
        match self {
            When::Date(s) | When::DateTime(s) => s,
        }
    }
}

/// Parse `YYYY-MM-DD` or `YYYY-MM-DDTHH:MM[:SS]` followed by `Z` or `+HH:MM/-HH:MM`.
/// Date-times are converted to UTC; a date-time without a zone is an error (there is no tz
/// database to guess one from).
pub fn parse_when(s: &str) -> Result<When> {
    let bad = || {
        err(format!(
            "invalid date/time `{s}`, expected YYYY-MM-DD or YYYY-MM-DDTHH:MM[:SS] with Z or +HH:MM/-HH:MM"
        ))
    };
    let Some((date, rest)) = s.split_once('T') else {
        check_date(s).map_err(|_| bad())?;
        return Ok(When::Date(s.to_string()));
    };
    check_date(date).map_err(|_| bad())?;
    let (time, zone) = rest.split_at(rest.find(['Z', '+', '-']).unwrap_or(rest.len()));
    let num = |p: &str, max: i64| {
        Some(p)
            .filter(|p| p.len() == 2 && p.bytes().all(|b| b.is_ascii_digit()))
            .and_then(|p| p.parse::<i64>().ok())
            .filter(|n| *n <= max)
            .ok_or_else(bad)
    };
    let (h, m, sec) = match time.split(':').collect::<Vec<_>>()[..] {
        [h, m] => (num(h, 23)?, num(m, 59)?, 0),
        [h, m, sec] => (num(h, 23)?, num(m, 59)?, num(sec, 59)?),
        _ => return Err(bad()),
    };
    // Offset in minutes east of UTC.
    let offset = match zone {
        "" => {
            return Err(err(format!(
                "date-time `{s}` has no time zone: add Z or an offset like +02:00"
            )));
        }
        "Z" => 0,
        z => {
            let sign = if z.starts_with('-') { -1 } else { 1 };
            let (oh, om) = z[1..].split_once(':').ok_or_else(bad)?;
            sign * (num(oh, 23)? * 60 + num(om, 59)?)
        }
    };
    let part = |r: std::ops::Range<usize>| date[r].parse::<i64>().map_err(|_| bad());
    let minutes =
        days_from_civil(part(0..4)?, part(5..7)?, part(8..10)?) * 1440 + h * 60 + m - offset;
    let (y, mo, d) = civil_from_days(minutes.div_euclid(1440));
    if !(1..=9999).contains(&y) {
        return Err(bad());
    }
    let min = minutes.rem_euclid(1440);
    Ok(When::DateTime(format!(
        "{y:04}-{mo:02}-{d:02}T{:02}:{:02}:{sec:02}Z",
        min / 60,
        min % 60
    )))
}

/// A "when" value in its stored form: task and milestone `due`, `datetime` attributes.
pub fn normalize_when(s: &str) -> Result<String> {
    parse_when(s).map(|w| w.as_str().to_string())
}

/// Days since 1970-01-01 for a proleptic Gregorian date (Howard Hinnant's algorithm).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

/// The inverse of `days_from_civil`.
fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dt(s: &str) -> String {
        match parse_when(s).unwrap() {
            When::DateTime(v) => v,
            w => panic!("{s}: not a date-time: {w:?}"),
        }
    }

    #[test]
    fn when_dates() {
        assert_eq!(
            parse_when("2026-10-22").unwrap(),
            When::Date("2026-10-22".into())
        );
        assert_eq!(parse_when("2024-02-29").unwrap().as_str(), "2024-02-29");
        assert!(parse_when("2026-02-29").is_err());
        assert!(parse_when("2026-10-22Z").is_err());
        assert!(parse_when("").is_err());
    }

    #[test]
    fn when_datetimes_normalize_to_utc() {
        assert_eq!(dt("2026-10-22T09:30Z"), "2026-10-22T09:30:00Z");
        assert_eq!(dt("2026-10-22T09:30:15Z"), "2026-10-22T09:30:15Z");
        assert_eq!(dt("2026-10-22T09:30+02:00"), "2026-10-22T07:30:00Z");
        assert_eq!(dt("2026-10-22T21:15-05:30"), "2026-10-23T02:45:00Z");
        assert_eq!(dt("2026-10-22T00:00+00:00"), "2026-10-22T00:00:00Z");
        // Across a day, month and year boundary.
        assert_eq!(dt("2027-01-01T01:00+02:00"), "2026-12-31T23:00:00Z");
        assert_eq!(dt("2026-12-31T23:00-01:00"), "2027-01-01T00:00:00Z");
        // Leap days, in both directions.
        assert_eq!(dt("2024-03-01T00:30+01:00"), "2024-02-29T23:30:00Z");
        assert_eq!(dt("2024-02-29T23:30-01:00"), "2024-03-01T00:30:00Z");
        assert_eq!(dt("2023-03-01T00:30+01:00"), "2023-02-28T23:30:00Z");
        assert_eq!(dt("2000-02-29T12:00Z"), "2000-02-29T12:00:00Z");
    }

    #[test]
    fn when_rejects_bad_datetimes() {
        let e = |s: &str| parse_when(s).unwrap_err().0;
        assert!(e("2026-10-22T09:30").contains("add Z or an offset"));
        assert!(e("2026-10-22T09:30:00").contains("add Z or an offset"));
        for s in [
            "2026-10-22T24:00Z",
            "2026-10-22T23:60Z",
            "2026-10-22T23:59:60Z",
            "2026-10-22T9:30Z",
            "2026-10-22T09Z",
            "2026-10-22T09:30:00:00Z",
            "2026-10-22T09:30+0200",
            "2026-10-22T09:30+24:00",
            "2026-10-22T09:30+02:00Z",
            "2026-10-22T09:30Zx",
            "2026-02-30T09:30Z",
            "2026-10-22T",
            "0001-01-01T00:00+01:00",
        ] {
            assert!(e(s).contains("invalid date/time"), "{s}");
        }
    }

    #[test]
    fn civil_days_round_trip() {
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(civil_from_days(-1), (1969, 12, 31));
        for z in [-800_000, -1, 0, 59, 11_016, 19_782, 2_000_000] {
            let (y, m, d) = civil_from_days(z);
            assert_eq!(days_from_civil(y, m, d), z);
        }
    }

    #[test]
    fn categories() {
        assert!(Category::Cancelled.is_closed());
        assert!(!Category::Active.is_closed());
    }

    #[test]
    fn dates() {
        assert!(check_date("2026-10-22").is_ok());
        assert!(check_date("2024-02-29").is_ok());
        assert!(check_date("2026-02-29").is_err());
        assert!(check_date("2026-13-01").is_err());
        assert!(check_date("2026-1-01").is_err());
        assert!(check_date("tomorrow").is_err());
    }

    #[test]
    fn pairs() {
        assert_eq!(
            parse_pair("size=a=b"),
            Ok(("size".to_string(), "a=b".to_string()))
        );
        assert!(parse_pair("=x").is_err());
        assert!(parse_pair("novalue").is_err());
    }
}
