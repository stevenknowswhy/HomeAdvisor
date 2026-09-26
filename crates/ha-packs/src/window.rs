//! Calendar windows: when a rule is live, and how far the next deadline
//! is.
//!
//! Windows compute from the calendar only — the variable input is the
//! date. Three kinds: `none` (always live), `annual` (a repeating
//! opens→closes span that may wrap the new year, like FAFSA's
//! October-to-June filing season), and `deadline` (one annual close date,
//! live from New Year's Day to the close, then dark until the next
//! cycle).

use chrono::{Datelike, NaiveDate};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// A month-and-day that recurs every year, written in the ISO reduced
/// precision form the pack data uses (`--MM-DD`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AnnualDate {
    month: u8,
    day: u8,
}

/// An annual date that is not a real calendar day (e.g. `--13-45`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("annual date --{month:0>2}-{day:0>2} is not a real calendar day")]
pub struct AnnualDateError {
    pub month: u8,
    pub day: u8,
}

impl AnnualDate {
    /// Checked constructor. February 29 is accepted (leap years exist);
    /// resolving it in a common year clamps to February 28.
    pub fn try_new(month: u8, day: u8) -> Result<Self, AnnualDateError> {
        // 2000 is a leap year, so it is the widest valid day grid.
        if NaiveDate::from_ymd_opt(2000, u32::from(month), u32::from(day)).is_none() {
            return Err(AnnualDateError { month, day });
        }
        Ok(Self { month, day })
    }

    pub fn month(self) -> u8 {
        self.month
    }

    pub fn day(self) -> u8 {
        self.day
    }

    /// The concrete date this month-day takes in `year`. February 29
    /// resolves to February 28 in common years — a recurring deadline is
    /// never skipped, only shifted.
    pub fn resolve(self, year: i32) -> NaiveDate {
        NaiveDate::from_ymd_opt(year, u32::from(self.month), u32::from(self.day))
            .or_else(|| NaiveDate::from_ymd_opt(year, 2, 28))
            .expect("a validated month/day resolves in every year")
    }
}

impl Serialize for AnnualDate {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&format!("--{:0>2}-{:0>2}", self.month, self.day))
    }
}

impl<'de> Deserialize<'de> for AnnualDate {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        let body = raw.strip_prefix("--").ok_or_else(|| {
            serde::de::Error::custom(format!("annual date must look like --MM-DD, got {raw:?}"))
        })?;
        let (month, day) = body.split_once('-').ok_or_else(|| {
            serde::de::Error::custom(format!("annual date must look like --MM-DD, got {raw:?}"))
        })?;
        let month: u8 = month
            .parse()
            .map_err(|_| serde::de::Error::custom(format!("bad month in annual date {raw:?}")))?;
        let day: u8 = day
            .parse()
            .map_err(|_| serde::de::Error::custom(format!("bad day in annual date {raw:?}")))?;
        AnnualDate::try_new(month, day).map_err(|e| serde::de::Error::custom(e.to_string()))
    }
}

/// When a rule is live. Parsed from the pack's `window` object; `None`
/// (always live) when the pack omits the window.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Window {
    /// Always live.
    #[default]
    None,
    /// A repeating span; may wrap the new year (October → June).
    Annual {
        opens: AnnualDate,
        closes: AnnualDate,
    },
    /// One annual close date: live from New Year's Day to the close, then
    /// dark until the next cycle — "day after closes" is inactive by
    /// construction.
    Deadline { closes: AnnualDate },
}

impl Window {
    /// Does this window have a close date? (A nudge is configured against
    /// it; the pack schema rejects a nudge without one.)
    pub fn close_date(self) -> Option<AnnualDate> {
        match self {
            Window::None => None,
            Window::Annual { closes, .. } | Window::Deadline { closes } => Some(closes),
        }
    }

    /// Is the window live on `today`?
    pub fn is_active(self, today: NaiveDate) -> bool {
        match self {
            Window::None => true,
            Window::Annual { opens, closes } => {
                let opens = opens.resolve(today.year());
                let closes = closes.resolve(today.year());
                if opens <= closes {
                    opens <= today && today <= closes
                } else {
                    // Wrapped (October → June): live from opens to year
                    // end and from year start to closes.
                    today >= opens || today <= closes
                }
            }
            Window::Deadline { closes } => closes.resolve(today.year()) >= today,
        }
    }

    /// Days from `today` to the next close (0 means today is the close
    /// date). `None` for a window with no close date.
    pub fn days_until_close(self, today: NaiveDate) -> Option<i64> {
        let closes = self.close_date()?;
        let mut year = today.year();
        let mut close = closes.resolve(year);
        if close < today {
            year += 1;
            close = closes.resolve(year);
        }
        Some((close - today).num_days())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).expect("test date")
    }

    #[test]
    fn annual_dates_reject_impossible_days() {
        assert!(AnnualDate::try_new(2, 29).is_ok());
        assert!(AnnualDate::try_new(2, 30).is_err());
        assert!(AnnualDate::try_new(13, 1).is_err());
        assert!(AnnualDate::try_new(0, 1).is_err());
        assert_eq!(
            AnnualDate::try_new(4, 31)
                .expect_err("impossible")
                .to_string(),
            "annual date --04-31 is not a real calendar day"
        );
    }

    #[test]
    fn annual_dates_roundtrip_through_the_iso_form() {
        let json = serde_json::to_string(&AnnualDate::try_new(10, 1).expect("valid")).unwrap();
        assert_eq!(json, "\"--10-01\"");
        let back: AnnualDate = serde_json::from_str(&json).unwrap();
        assert_eq!(back, AnnualDate::try_new(10, 1).unwrap());
        assert_eq!(
            serde_json::to_string(&AnnualDate::try_new(2, 7).unwrap()).unwrap(),
            "\"--02-07\""
        );
        // Malformed forms are hard parse errors, not silent misreads.
        for bad in [
            "2026-10-01",
            "10-01",
            "--13-01",
            "--00-10",
            "--10-32",
            "october",
        ] {
            let result: Result<AnnualDate, _> = serde_json::from_str(&format!("\"{bad}\""));
            assert!(result.is_err(), "expected {bad:?} to be rejected");
        }
    }

    #[test]
    fn feb29_clamps_to_feb28_in_common_years() {
        assert_eq!(
            AnnualDate::try_new(2, 29).unwrap().resolve(2027),
            date(2027, 2, 28)
        );
        assert_eq!(
            AnnualDate::try_new(2, 29).unwrap().resolve(2028),
            date(2028, 2, 29)
        );
    }

    #[test]
    fn annual_window_boundaries_across_the_new_year() {
        // FAFSA's cycle: opens October 1, closes June 30.
        let fafsa = Window::Annual {
            opens: AnnualDate::try_new(10, 1).unwrap(),
            closes: AnnualDate::try_new(6, 30).unwrap(),
        };
        assert!(!fafsa.is_active(date(2026, 9, 30)), "day before opens");
        assert!(fafsa.is_active(date(2026, 10, 1)), "opens");
        assert!(fafsa.is_active(date(2026, 12, 31)), "spans the new year");
        assert!(fafsa.is_active(date(2027, 6, 30)), "close date");
        assert!(!fafsa.is_active(date(2027, 7, 1)), "day after closes");
    }

    #[test]
    fn annual_window_within_one_year() {
        let span = Window::Annual {
            opens: AnnualDate::try_new(9, 1).unwrap(),
            closes: AnnualDate::try_new(10, 31).unwrap(),
        };
        assert!(!span.is_active(date(2026, 8, 31)));
        assert!(span.is_active(date(2026, 9, 1)));
        assert!(span.is_active(date(2026, 10, 31)));
        assert!(!span.is_active(date(2026, 11, 1)));
    }

    #[test]
    fn deadline_window_boundaries() {
        let tax_day = Window::Deadline {
            closes: AnnualDate::try_new(4, 15).unwrap(),
        };
        assert!(
            tax_day.is_active(date(2026, 1, 1)),
            "live from New Year's Day"
        );
        assert!(tax_day.is_active(date(2026, 4, 14)), "day before close");
        assert!(tax_day.is_active(date(2026, 4, 15)), "close date");
        assert!(!tax_day.is_active(date(2026, 4, 16)), "day after closes");
        assert!(
            !tax_day.is_active(date(2026, 12, 31)),
            "dark until the next cycle"
        );
    }

    #[test]
    fn days_until_close_counts_across_the_year_boundary() {
        let fafsa = Window::Annual {
            opens: AnnualDate::try_new(10, 1).unwrap(),
            closes: AnnualDate::try_new(6, 30).unwrap(),
        };
        assert_eq!(fafsa.days_until_close(date(2026, 10, 1)), Some(272));
        assert_eq!(fafsa.days_until_close(date(2026, 12, 31)), Some(181));
        assert_eq!(fafsa.days_until_close(date(2027, 6, 30)), Some(0));
        assert_eq!(fafsa.days_until_close(date(2027, 7, 1)), Some(365)); // 2028 is a leap year
        let none = Window::None;
        assert_eq!(none.days_until_close(date(2026, 10, 1)), None);
    }

    #[test]
    fn close_date_is_none_only_for_the_open_window() {
        assert_eq!(Window::None.close_date(), None);
        assert_eq!(
            Window::Deadline {
                closes: AnnualDate::try_new(4, 15).unwrap()
            }
            .close_date(),
            AnnualDate::try_new(4, 15).ok()
        );
    }
}
