//! Values shared by every command group: item status and priority, and `name=value` pairs.

use clap::ValueEnum;

use crate::error::{Result, err};

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum Status {
    Todo,
    Doing,
    Blocked,
    Done,
    Cancelled,
}

impl Status {
    pub fn as_str(self) -> &'static str {
        match self {
            Status::Todo => "todo",
            Status::Doing => "doing",
            Status::Blocked => "blocked",
            Status::Done => "done",
            Status::Cancelled => "cancelled",
        }
    }

    /// Done and cancelled items are hidden from `list` unless `--all`.
    pub fn is_closed(self) -> bool {
        matches!(self, Status::Done | Status::Cancelled)
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

#[cfg(test)]
mod tests {
    use super::*;

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
