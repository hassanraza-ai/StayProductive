//! Period arithmetic and local-time resolution.
//!
//! All daily boundaries are computed from **local dates**, never by adding
//! 86,400 seconds, so DST transitions and a non-midnight start-of-day behave
//! correctly (see `docs/spec/domain-model.md` §5 and
//! `docs/spec/schedules-and-conditions.md` §2).
//!
//! The caller supplies a `chrono_tz::Tz`, which follows the system time zone
//! when the daemon reloads it. No function here reads the wall clock.

use chrono::{
    DateTime, Days, LocalResult, NaiveDate, NaiveDateTime, NaiveTime, TimeDelta, TimeZone,
    Timelike, Utc,
};
use chrono_tz::Tz;
use serde::{Deserialize, Serialize};

use crate::time::Timestamp;

/// Clock configuration that affects period boundaries.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClockSettings {
    /// Local time at which the logical day starts.
    pub start_of_day: NaiveTime,
    /// IANA time zone used for local-time calculations.
    pub timezone: Tz,
}

impl Default for ClockSettings {
    fn default() -> Self {
        Self {
            start_of_day: NaiveTime::MIN,
            timezone: chrono_tz::UTC,
        }
    }
}

impl ClockSettings {
    /// Create settings.
    pub fn new(start_of_day: NaiveTime, timezone: Tz) -> Self {
        Self {
            start_of_day,
            timezone,
        }
    }

    /// Settings using midnight in `timezone`.
    pub fn midnight_in(timezone: Tz) -> Self {
        Self::new(NaiveTime::MIN, timezone)
    }

    /// Settings with an explicit start-of-day offset in `timezone`.
    pub fn with_start_of_day(start_of_day: NaiveTime, timezone: Tz) -> Self {
        Self::new(start_of_day, timezone)
    }
}

/// A half-open period `[start, end)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Period {
    /// Inclusive start.
    pub start: Timestamp,
    /// Exclusive end.
    pub end: Timestamp,
}

impl Period {
    /// Create a period.
    pub const fn new(start: Timestamp, end: Timestamp) -> Self {
        Self { start, end }
    }

    /// Whether `at` lies inside the period.
    pub fn contains(&self, at: Timestamp) -> bool {
        self.start <= at && at < self.end
    }

    /// The length of the period. DST days are 23 or 25 hours.
    pub fn duration(&self) -> TimeDelta {
        self.end - self.start
    }

    /// Clip an interval to this period.
    pub fn clip(&self, start: Timestamp, end: Timestamp) -> Option<(Timestamp, Timestamp)> {
        let start = start.max(self.start);
        let end = end.min(self.end);
        (start < end).then_some((start, end))
    }
}

/// Resolve a local wall-clock time in `tz` to a UTC instant.
///
/// - Ambiguous times (DST fall-back) resolve to the **earliest** occurrence, so
///   a repeated hour is evaluated once.
/// - Non-existent times (DST spring-forward) resolve to the first valid instant
///   after the gap. A window entirely inside the gap therefore collapses to
///   zero length and does not run.
pub fn resolve_local(local: NaiveDateTime, tz: Tz) -> Option<Timestamp> {
    match tz.from_local_datetime(&local) {
        LocalResult::Single(dt) => Some(dt.with_timezone(&Utc)),
        LocalResult::Ambiguous(earliest, _) => Some(earliest.with_timezone(&Utc)),
        LocalResult::None => resolve_gap(local, tz),
    }
}

/// Resolve a wall-clock time that falls inside a DST gap.
///
/// The request is shifted forward by the length of the gap, so 02:30 on a
/// spring-forward day becomes 03:30 and keeps its position within the local
/// hour. A window that both starts and ends inside the gap still collapses to
/// zero length.
fn resolve_gap(local: NaiveDateTime, tz: Tz) -> Option<Timestamp> {
    let mut first_valid_local = None;
    for minutes in 1..=(6 * 60) {
        let candidate = local + TimeDelta::minutes(minutes);
        if tz.from_local_datetime(&candidate).earliest().is_some() {
            first_valid_local = Some(candidate);
            break;
        }
    }
    let first_valid_local = first_valid_local?;
    let mut last_valid_local = None;
    for minutes in 1..=(6 * 60) {
        let candidate = local - TimeDelta::minutes(minutes);
        if tz.from_local_datetime(&candidate).earliest().is_some() {
            last_valid_local = Some(candidate);
            break;
        }
    }
    let Some(last_valid_local) = last_valid_local else {
        return tz
            .from_local_datetime(&first_valid_local)
            .earliest()
            .map(|dt| dt.with_timezone(&Utc));
    };
    let gap_start = last_valid_local + TimeDelta::minutes(1);
    let gap = first_valid_local - gap_start;
    tz.from_local_datetime(&(local + gap))
        .earliest()
        .or_else(|| tz.from_local_datetime(&first_valid_local).earliest())
        .map(|dt| dt.with_timezone(&Utc))
}

/// Resolve a local date and time in `tz` to a UTC instant.
pub fn resolve_date_time(date: NaiveDate, time: NaiveTime, tz: Tz) -> Option<Timestamp> {
    resolve_local(date.and_time(time), tz)
}

/// The logical date an instant belongs to.
///
/// A start-of-day of 04:00 means 02:00 on Tuesday belongs to Monday.
pub fn logical_date(now: Timestamp, settings: &ClockSettings) -> NaiveDate {
    let local = now.with_timezone(&settings.timezone).naive_local();
    let offset = TimeDelta::seconds(
        settings
            .start_of_day
            .signed_duration_since(NaiveTime::MIN)
            .num_seconds(),
    );
    (local - offset).date()
}

/// The daily period containing `now`.
pub fn day_period(now: Timestamp, settings: &ClockSettings) -> Period {
    let date = logical_date(now, settings);
    day_period_for(date, settings)
}

/// The daily period for a logical date.
pub fn day_period_for(date: NaiveDate, settings: &ClockSettings) -> Period {
    let start_date = date;
    let end_date = date.checked_add_days(Days::new(1)).unwrap_or(date);
    let start = resolve_date_time(start_date, settings.start_of_day, settings.timezone)
        .unwrap_or_else(|| now_fallback(start_date, settings));
    let end = resolve_date_time(end_date, settings.start_of_day, settings.timezone)
        .unwrap_or_else(|| start + TimeDelta::days(1));
    Period::new(start, end.max(start + TimeDelta::seconds(1)))
}

/// The next daily boundary after `now`.
pub fn next_day_boundary(now: Timestamp, settings: &ClockSettings) -> Timestamp {
    day_period(now, settings).end
}

/// The local wall-clock hour containing `now`, expressed as a UTC period.
///
/// A skipped hour collapses to an empty period and therefore records no usage;
/// a repeated hour is anchored at its first occurrence and counted once.
pub fn hour_period(now: Timestamp, timezone: Tz) -> Period {
    let local = now.with_timezone(&timezone);
    let date = local.date_naive();
    let hour_start = date
        .and_hms_opt(local.hour(), 0, 0)
        .unwrap_or(local.naive_local());
    let start = resolve_local(hour_start, timezone).unwrap_or(now);
    let next_local = hour_start + TimeDelta::hours(1);
    let end = resolve_local(next_local, timezone).unwrap_or(start + TimeDelta::hours(1));
    Period::new(start, end.max(start + TimeDelta::seconds(1)))
}

/// The next local-hour boundary after `now`.
pub fn next_hour_boundary(now: Timestamp, timezone: Tz) -> Timestamp {
    hour_period(now, timezone).end
}

/// Whether two wall-clock times are in the same local hour.
pub fn same_local_hour(a: Timestamp, b: Timestamp, timezone: Tz) -> bool {
    let a = a.with_timezone(&timezone);
    let b = b.with_timezone(&timezone);
    a.date_naive() == b.date_naive() && a.hour() == b.hour()
}

fn now_fallback(date: NaiveDate, settings: &ClockSettings) -> Timestamp {
    // Only reachable if a time zone database is incomplete. Treat the local
    // wall time as UTC so the period stays well-formed; callers surface the
    // degraded state.
    DateTime::<Utc>::from_naive_utc_and_offset(date.and_time(settings.start_of_day), Utc)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use chrono_tz::America::New_York;

    fn local(tz: Tz, y: i32, m: u32, d: u32, h: u32, min: u32, s: u32) -> Timestamp {
        let naive = NaiveDate::from_ymd_opt(y, m, d)
            .unwrap()
            .and_hms_opt(h, min, s)
            .unwrap();
        resolve_local(naive, tz).unwrap()
    }

    #[test]
    fn logical_day_honours_start_of_day() {
        let settings = ClockSettings::with_start_of_day(
            NaiveTime::from_hms_opt(4, 0, 0).unwrap(),
            chrono_tz::UTC,
        );
        let tuesday_2am = Utc.with_ymd_and_hms(2026, 10, 20, 2, 0, 0).unwrap();
        assert_eq!(
            logical_date(tuesday_2am, &settings),
            NaiveDate::from_ymd_opt(2026, 10, 19).unwrap()
        );
        let tuesday_4am = Utc.with_ymd_and_hms(2026, 10, 20, 4, 0, 0).unwrap();
        assert_eq!(
            logical_date(tuesday_4am, &settings),
            NaiveDate::from_ymd_opt(2026, 10, 20).unwrap()
        );
    }

    #[test]
    fn day_period_starts_at_start_of_day() {
        let settings = ClockSettings::with_start_of_day(
            NaiveTime::from_hms_opt(4, 0, 0).unwrap(),
            chrono_tz::UTC,
        );
        let tuesday_2am = Utc.with_ymd_and_hms(2026, 10, 20, 2, 0, 0).unwrap();
        let p = day_period(tuesday_2am, &settings);
        assert_eq!(
            p.start,
            Utc.with_ymd_and_hms(2026, 10, 19, 4, 0, 0).unwrap()
        );
        assert_eq!(p.end, Utc.with_ymd_and_hms(2026, 10, 20, 4, 0, 0).unwrap());
        assert!(p.contains(tuesday_2am));
    }

    #[test]
    fn dst_spring_day_is_twenty_three_hours() {
        let settings = ClockSettings::midnight_in(New_York);
        let now = local(New_York, 2026, 3, 8, 12, 0, 0);
        let p = day_period(now, &settings);
        assert_eq!(p.duration(), TimeDelta::hours(23));
    }

    #[test]
    fn dst_fall_day_is_twenty_five_hours() {
        let settings = ClockSettings::midnight_in(New_York);
        let now = local(New_York, 2026, 11, 1, 12, 0, 0);
        let p = day_period(now, &settings);
        assert_eq!(p.duration(), TimeDelta::hours(25));
    }

    #[test]
    fn skipped_local_time_resolves_forward() {
        // 02:30 does not exist on 2026-03-08 in New York.
        let resolved = resolve_local(
            NaiveDate::from_ymd_opt(2026, 3, 8)
                .unwrap()
                .and_hms_opt(2, 30, 0)
                .unwrap(),
            New_York,
        )
        .unwrap();
        // 03:30 EDT == 07:30 UTC.
        assert_eq!(
            resolved,
            Utc.with_ymd_and_hms(2026, 3, 8, 7, 30, 0).unwrap()
        );
    }

    #[test]
    fn hourly_period_bounds_are_half_open() {
        let now = Utc.with_ymd_and_hms(2026, 10, 22, 9, 59, 59).unwrap();
        let p = hour_period(now, chrono_tz::UTC);
        assert_eq!(
            p.start,
            Utc.with_ymd_and_hms(2026, 10, 22, 9, 0, 0).unwrap()
        );
        assert_eq!(p.end, Utc.with_ymd_and_hms(2026, 10, 22, 10, 0, 0).unwrap());
        assert!(p.contains(now));
        assert!(!p.contains(Utc.with_ymd_and_hms(2026, 10, 22, 10, 0, 0).unwrap()));
    }
}
