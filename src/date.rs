//! A minimal civil date.
//!
//! baselinefit only ever needs three things from a calendar: parse the
//! `YYYY-MM-DD` strings that web-features uses for `baseline_low_date` and
//! `baseline_high_date`, print them back, and compare them. That is a few dozen
//! lines, so it lives here rather than pulling in a date crate (see CONTRIBUTING
//! for the dependency policy).
//!
//! There is deliberately no "today". Every verdict baselinefit reaches is a
//! function of the embedded snapshot and the `--target` flag, never of the
//! clock, so two runs of the same binary over the same tree always agree.

use std::fmt;

/// A calendar date with no time and no zone.
///
/// Field order matters: the derived `Ord` compares year, then month, then day,
/// which is chronological.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Date {
    year: i32,
    month: u32,
    day: u32,
}

impl Date {
    /// Build a date, rejecting out-of-range months and days (leap years
    /// included).
    pub fn new(year: i32, month: u32, day: u32) -> Option<Date> {
        if !(1..=12).contains(&month) || day == 0 || day > days_in_month(year, month) {
            return None;
        }
        Some(Date { year, month, day })
    }

    /// Parse a strict `YYYY-MM-DD` string. Anything looser is an error: the
    /// dataset only ever emits this form, so accepting more would only let a
    /// corrupted snapshot through.
    pub fn parse(s: &str) -> Result<Date, String> {
        let bytes = s.as_bytes();
        let shaped = bytes.len() == 10
            && bytes[4] == b'-'
            && bytes[7] == b'-'
            && bytes
                .iter()
                .enumerate()
                .all(|(i, b)| i == 4 || i == 7 || b.is_ascii_digit());
        if !shaped {
            return Err(format!("expected a date as YYYY-MM-DD, got '{s}'"));
        }
        let year: i32 = s[0..4].parse().map_err(|_| bad(s))?;
        let month: u32 = s[5..7].parse().map_err(|_| bad(s))?;
        let day: u32 = s[8..10].parse().map_err(|_| bad(s))?;
        Date::new(year, month, day).ok_or_else(|| format!("'{s}' is not a real calendar date"))
    }

    pub fn year(self) -> i32 {
        self.year
    }

    pub fn month(self) -> u32 {
        self.month
    }

    pub fn day(self) -> u32 {
        self.day
    }
}

fn bad(s: &str) -> String {
    format!("expected a date as YYYY-MM-DD, got '{s}'")
}

fn is_leap(year: i32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

fn days_in_month(year: i32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap(year) => 29,
        2 => 28,
        _ => 0,
    }
}

impl fmt::Display for Date {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:04}-{:02}-{:02}", self.year, self.month, self.day)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_prints_round_trip() {
        let d = Date::parse("2026-06-19").unwrap();
        assert_eq!(d.to_string(), "2026-06-19");
        assert_eq!((d.year(), d.month(), d.day()), (2026, 6, 19));
    }

    #[test]
    fn rejects_malformed_and_impossible_dates() {
        for bad in [
            "2026-6-19",
            "26-06-19",
            "2026/06/19",
            "",
            "2026-06-19T00:00:00Z",
            "2026-13-01",
            "2026-00-10",
            "2026-02-30",
            "2026-04-31",
            "2026-01-00",
        ] {
            assert!(Date::parse(bad).is_err(), "{bad} should not parse");
        }
        assert!(Date::parse("2024-02-29").is_ok(), "2024 is a leap year");
        assert!(Date::parse("2026-02-29").is_err(), "2026 is not");
        assert!(Date::parse("2000-02-29").is_ok(), "2000 is a leap year");
        assert!(Date::parse("1900-02-29").is_err(), "1900 is not");
    }

    #[test]
    fn ordering_is_chronological() {
        let a = Date::parse("2023-02-14").unwrap();
        let b = Date::parse("2023-12-19").unwrap();
        let c = Date::parse("2025-01-02").unwrap();
        assert!(a < b);
        assert!(b < c);
        assert!(a < c);
        assert_eq!(a.min(c), a);
    }
}
