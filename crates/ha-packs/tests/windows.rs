//! AC1 deadline-window boundary tests: day before opens, open date,
//! close date, day after closes, the 48-hour edge, and the year wrap.
//! Both synthetic windows and the shipped packs' real windows are pinned.

use chrono::NaiveDate;
use ha_packs::{built_in_packs, AnnualDate, Pack, Window};

fn date(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).expect("test date")
}

fn rule<'a>(pack: &'a Pack, id: &str) -> &'a ha_packs::Rule {
    pack.rules()
        .iter()
        .find(|r| r.id == id)
        .unwrap_or_else(|| panic!("{id} exists in the shipped pack"))
}

#[test]
fn shipped_fafsa_window_boundaries() {
    let packs = built_in_packs().expect("packs parse");
    let education = packs
        .iter()
        .find(|p| p.id().as_str() == "education")
        .unwrap();
    let fafsa = rule(education, "education.fafsa-window-open");
    let window = fafsa.when.window;

    // The FAFSA cycle runs October 1 to June 30 — it wraps the new year.
    assert!(!window.is_active(date(2026, 9, 30)), "day before opens");
    assert!(window.is_active(date(2026, 10, 1)), "opens");
    assert!(window.is_active(date(2027, 6, 30)), "close date");
    assert!(!window.is_active(date(2027, 7, 1)), "day after closes");
    assert_eq!(window.days_until_close(date(2026, 10, 1)), Some(272));
    assert_eq!(window.days_until_close(date(2027, 6, 30)), Some(0));
    assert_eq!(window.days_until_close(date(2027, 7, 1)), Some(365)); // 2028 is a leap year
}

#[test]
fn shipped_deadline_windows_light_up_and_go_dark() {
    let packs = built_in_packs().expect("packs parse");
    let finance = packs.iter().find(|p| p.id().as_str() == "finance").unwrap();

    let ctc = rule(finance, "finance.ctc-claim").when.window;
    assert!(ctc.is_active(date(2026, 1, 1)), "live from New Year's Day");
    assert!(
        !ctc.is_active(date(2025, 12, 31)),
        "dark until the next cycle"
    );
    assert!(ctc.is_active(date(2027, 4, 15)), "close date");
    assert!(!ctc.is_active(date(2027, 4, 16)), "day after closes");

    let fsa = rule(finance, "finance.fsa-year-end").when.window;
    // A December 31 close is the degenerate always-live deadline.
    assert!(fsa.is_active(date(2026, 7, 15)));
    assert_eq!(fsa.days_until_close(date(2026, 7, 15)), Some(169));
    assert_eq!(fsa.days_until_close(date(2026, 12, 31)), Some(0));
    assert_eq!(fsa.days_until_close(date(2027, 1, 1)), Some(364));
}

#[test]
fn shipped_marketplace_window_wraps_the_year() {
    let packs = built_in_packs().expect("packs parse");
    let finance = packs.iter().find(|p| p.id().as_str() == "finance").unwrap();
    let window = rule(finance, "finance.marketplace-open-enrollment")
        .when
        .window;

    assert!(!window.is_active(date(2026, 10, 31)), "day before opens");
    assert!(window.is_active(date(2026, 11, 1)), "opens");
    assert!(window.is_active(date(2026, 12, 31)), "spans the new year");
    assert!(window.is_active(date(2027, 1, 15)), "close date");
    assert!(!window.is_active(date(2027, 1, 16)), "day after closes");
    assert_eq!(window.days_until_close(date(2026, 12, 20)), Some(26));
}

#[test]
fn the_48_hour_nudge_edge() {
    // The built-in FSA rule nudges at [14, 2] — the 48-hour rung is `2`.
    let packs = built_in_packs().expect("packs parse");
    let finance = packs.iter().find(|p| p.id().as_str() == "finance").unwrap();
    let fsa = rule(finance, "finance.fsa-year-end");

    let check = |day: NaiveDate| {
        (
            day,
            fsa.when.window.days_until_close(day),
            fsa.nudge_due(day),
        )
    };

    // Outside the rungs: 15 and 3 and 1 days out → no nudge.
    assert_eq!(
        check(date(2026, 12, 16)),
        (date(2026, 12, 16), Some(15), None)
    );
    assert_eq!(
        check(date(2026, 12, 28)),
        (date(2026, 12, 28), Some(3), None)
    );
    assert_eq!(
        check(date(2026, 12, 30)),
        (date(2026, 12, 30), Some(1), None)
    );
    // On the rungs: 14 days and 2 days (the 48-hour edge).
    assert_eq!(
        check(date(2026, 12, 17)),
        (date(2026, 12, 17), Some(14), Some(14))
    );
    assert_eq!(
        check(date(2026, 12, 29)),
        (date(2026, 12, 29), Some(2), Some(2))
    );
    // Close day itself: past the 48-hour rung, no nudge.
    assert_eq!(
        check(date(2026, 12, 31)),
        (date(2026, 12, 31), Some(0), None)
    );
}

#[test]
fn synthetic_annual_window_boundaries_within_one_year() {
    let span = Window::Annual {
        opens: AnnualDate::try_new(2, 1).unwrap(),
        closes: AnnualDate::try_new(4, 30).unwrap(),
    };
    assert!(!span.is_active(date(2027, 1, 31)));
    assert!(span.is_active(date(2027, 2, 1)));
    assert!(span.is_active(date(2027, 4, 30)));
    assert!(!span.is_active(date(2027, 5, 1)));
}

#[test]
fn deadline_days_until_close_never_reports_negative() {
    let deadline = Window::Deadline {
        closes: AnnualDate::try_new(4, 15).unwrap(),
    };
    assert_eq!(deadline.days_until_close(date(2026, 4, 15)), Some(0));
    // Day after close rolls to next year's close, 364 days out.
    assert_eq!(deadline.days_until_close(date(2026, 4, 16)), Some(364));
}
