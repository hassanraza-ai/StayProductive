//! Schedules: when a profile's rules are active.
//!
//! A schedule compiles to half-open UTC intervals for a given local date. Rules
//! for cross-midnight windows belong to the day the window starts; evaluation
//! therefore looks at today and yesterday (see
//! `docs/spec/schedules-and-conditions.md` §3).

use std::time::Duration;

use chrono::{Datelike, Days, NaiveDate, NaiveDateTime, NaiveTime, TimeDelta, Timelike, Weekday};
use chrono_tz::Tz;
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::period::{Period, resolve_date_time};
use crate::time::Timestamp;

/// A local time range `[start, end)`.
///
/// `end < start` crosses midnight; `start == end` is a full local day (24 h) and
/// is only valid for [`ScheduleKind::FullDay`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Window {
    /// Inclusive local start.
    pub start: NaiveTime,
    /// Exclusive local end. May be earlier than `start`.
    pub end: NaiveTime,
}

impl Window {
    /// Create a window.
    pub fn new(start: NaiveTime, end: NaiveTime) -> Self {
        Self { start, end }
    }

    /// A window covering the entire local day.
    pub const fn full_day() -> Self {
        Self {
            start: NaiveTime::MIN,
            end: NaiveTime::MIN,
        }
    }

    /// Whether this window spans 24 hours.
    pub fn is_full_day(self) -> bool {
        self.start == self.end
    }

    /// Whether the window starts on one date and ends on the next.
    pub fn crosses_midnight(self) -> bool {
        self.end <= self.start
    }

    /// Length in local wall-clock time.
    pub fn local_duration(self) -> TimeDelta {
        if self.is_full_day() {
            TimeDelta::days(1)
        } else if self.end < self.start {
            TimeDelta::hours(24) - (self.start - self.end)
        } else {
            self.end - self.start
        }
    }

    fn interval_for(self, date: NaiveDate) -> Option<(NaiveDateTime, NaiveDateTime)> {
        let start = date.and_time(self.start);
        let end_date = if self.crosses_midnight() {
            date.checked_add_days(Days::new(1))?
        } else {
            date
        };
        Some((start, end_date.and_time(self.end)))
    }
}

impl Default for Window {
    fn default() -> Self {
        Self::full_day()
    }
}

/// Which local dates a schedule applies to.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum DayPattern {
    /// Every local date.
    #[default]
    EveryDay,
    /// Only the listed weekdays, using the schedule's shared windows.
    Weekdays(Vec<Weekday>),
    /// Per-weekday windows.
    Custom(Vec<(Weekday, Vec<Window>)>),
}

impl DayPattern {
    /// Monday through Friday.
    pub fn weekdays() -> Self {
        Self::Weekdays(vec![
            Weekday::Mon,
            Weekday::Tue,
            Weekday::Wed,
            Weekday::Thu,
            Weekday::Fri,
        ])
    }

    /// A single weekday.
    pub fn day(day: Weekday) -> Self {
        Self::Weekdays(vec![day])
    }

    /// The windows for `date` if this pattern applies, otherwise `None`.
    pub fn windows_for<'a>(
        &'a self,
        date: NaiveDate,
        shared: &'a [Window],
    ) -> Option<&'a [Window]> {
        match self {
            Self::EveryDay => Some(shared),
            Self::Weekdays(days) => days.contains(&date.weekday()).then_some(shared),
            Self::Custom(days) => days
                .iter()
                .find(|(day, _)| *day == date.weekday())
                .map(|(_, windows)| windows.as_slice()),
        }
    }

    fn applies(&self, date: NaiveDate) -> bool {
        match self {
            Self::EveryDay => true,
            Self::Weekdays(days) => days.contains(&date.weekday()),
            Self::Custom(days) => days.iter().any(|(day, _)| *day == date.weekday()),
        }
    }
}

/// Repeating within-hour pattern, e.g. 50 minutes on and 10 off.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HourlyPattern {
    /// Time allowed at the top of each hour.
    pub on: Duration,
    /// Time blocked before the next hour.
    pub off: Duration,
}

impl HourlyPattern {
    /// Create a pattern; `on + off` must equal one hour.
    pub fn new(on: Duration, off: Duration) -> Result<Self> {
        if on.is_zero() || on > Duration::from_secs(3600) {
            return Err(Error::InvalidSchedule(
                "hourly on-duration must be between 1 s and 1 h".into(),
            ));
        }
        if on + off != Duration::from_secs(3600) {
            return Err(Error::InvalidSchedule(
                "hourly on + off must equal one hour".into(),
            ));
        }
        Ok(Self { on, off })
    }

    /// 50 minutes on, 10 minutes off.
    pub fn fifty_ten() -> Self {
        Self {
            on: Duration::from_secs(50 * 60),
            off: Duration::from_secs(10 * 60),
        }
    }
}

/// An inclusive local date range for one-time schedules.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct DateRange {
    /// First local date.
    pub start: NaiveDate,
    /// Last local date (inclusive).
    pub end: NaiveDate,
}

impl DateRange {
    /// Create a range; `end` must not precede `start`.
    pub fn new(start: NaiveDate, end: NaiveDate) -> Result<Self> {
        if end < start {
            return Err(Error::InvalidSchedule(
                "date range ends before it starts".into(),
            ));
        }
        Ok(Self { start, end })
    }

    /// Whether the date is inside the range.
    pub fn contains(self, date: NaiveDate) -> bool {
        self.start <= date && date <= self.end
    }
}

/// The presentation kind of a schedule.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ScheduleKind {
    /// The same window every day.
    Daily,
    /// Chosen weekdays with one window set.
    Days,
    /// Several windows per day.
    Intervals,
    /// A whole local day.
    FullDay,
    /// A repeating within-hour pattern.
    Hourly,
    /// A dated range.
    OneTime,
}

/// A compiled schedule.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Schedule {
    /// Presentation kind; evaluation also uses the window data.
    pub kind: ScheduleKind,
    /// Shared windows, used by `Daily`, `Days`, `Intervals` and `OneTime`.
    pub windows: Vec<Window>,
    /// Which local dates apply.
    pub days: DayPattern,
    /// Optional one-time range.
    pub date_range: Option<DateRange>,
    /// Present for `Hourly`.
    pub hourly: Option<HourlyPattern>,
    /// Per-profile start-of-day override, used by the period calculator.
    pub start_of_day: Option<NaiveTime>,
}

impl Schedule {
    /// A schedule that runs the same window every day.
    pub fn daily(windows: Vec<Window>) -> Result<Self> {
        Self::build(
            ScheduleKind::Daily,
            windows,
            DayPattern::EveryDay,
            None,
            None,
        )
    }

    /// A schedule that runs on selected weekdays.
    pub fn days(days: DayPattern, windows: Vec<Window>) -> Result<Self> {
        Self::build(ScheduleKind::Days, windows, days, None, None)
    }

    /// A schedule with several windows per day.
    pub fn intervals(windows: Vec<Window>) -> Result<Self> {
        Self::build(
            ScheduleKind::Intervals,
            windows,
            DayPattern::EveryDay,
            None,
            None,
        )
    }

    /// A full-local-day schedule on selected days.
    pub fn full_day(days: DayPattern) -> Result<Self> {
        Self::build(
            ScheduleKind::FullDay,
            vec![Window::full_day()],
            days,
            None,
            None,
        )
    }

    /// A repeating within-hour schedule on selected days.
    pub fn hourly(days: DayPattern, pattern: HourlyPattern) -> Result<Self> {
        Self::build(ScheduleKind::Hourly, Vec::new(), days, None, Some(pattern))
    }

    /// A dated schedule.
    pub fn one_time(range: DateRange, windows: Vec<Window>) -> Result<Self> {
        Self::build(
            ScheduleKind::OneTime,
            windows,
            DayPattern::EveryDay,
            Some(range),
            None,
        )
    }

    fn build(
        kind: ScheduleKind,
        windows: Vec<Window>,
        days: DayPattern,
        date_range: Option<DateRange>,
        hourly: Option<HourlyPattern>,
    ) -> Result<Self> {
        let schedule = Self {
            kind,
            windows,
            days,
            date_range,
            hourly,
            start_of_day: None,
        };
        schedule.validate()?;
        Ok(schedule)
    }

    /// Override the start-of-day used by this profile's period calculations.
    pub fn with_start_of_day(mut self, start_of_day: NaiveTime) -> Self {
        self.start_of_day = Some(start_of_day);
        self
    }

    /// Validate the schedule, returning a field-level error.
    pub fn validate(&self) -> Result<()> {
        match self.kind {
            ScheduleKind::Hourly => {
                if self.hourly.is_none() {
                    return Err(Error::InvalidSchedule(
                        "hourly schedule requires an hourly pattern".into(),
                    ));
                }
                if !self.windows.is_empty() {
                    return Err(Error::InvalidSchedule(
                        "hourly schedule cannot also have windows".into(),
                    ));
                }
            }
            ScheduleKind::OneTime => {
                if self.date_range.is_none() {
                    return Err(Error::InvalidSchedule(
                        "one-time schedule requires a date range".into(),
                    ));
                }
                if self.windows.is_empty() {
                    return Err(Error::InvalidSchedule(
                        "schedule needs at least one window".into(),
                    ));
                }
            }
            _ => {
                if self.windows.is_empty() {
                    return Err(Error::InvalidSchedule(
                        "schedule needs at least one window".into(),
                    ));
                }
            }
        }

        if matches!(self.days, DayPattern::Custom(ref days) if days.iter().any(|(_, windows)| windows.is_empty()))
        {
            return Err(Error::InvalidSchedule(
                "custom days must each have at least one window".into(),
            ));
        }

        for window in &self.windows {
            if window.start.second() != 0 || window.end.second() != 0 {
                return Err(Error::InvalidSchedule(
                    "windows use minute resolution".into(),
                ));
            }
            if window.is_full_day() && self.kind != ScheduleKind::FullDay {
                return Err(Error::InvalidSchedule(
                    "start and end may only match for a full-day schedule".into(),
                ));
            }
            if window.local_duration() > TimeDelta::hours(24) {
                return Err(Error::InvalidSchedule(
                    "a window cannot exceed 24 hours".into(),
                ));
            }
        }
        Ok(())
    }

    /// Whether the schedule applies on a local date.
    pub fn applies_on(&self, date: NaiveDate) -> bool {
        if let Some(range) = self.date_range {
            if !range.contains(date) {
                return false;
            }
        }
        self.days.applies(date)
    }

    /// Evaluate the schedule at `now` in `tz`.
    pub fn status(&self, now: Timestamp, tz: Tz) -> ScheduleStatus {
        if let Some(pattern) = self.hourly {
            return self.hourly_status(now, tz, pattern);
        }
        let intervals = self.intervals_around(now, tz);
        let active = intervals
            .iter()
            .find(|(start, end)| *start <= now && now < *end)
            .copied();
        match active {
            Some((_, end)) => ScheduleStatus {
                active: true,
                ends_at: Some(end),
                next_start: None,
            },
            None => ScheduleStatus {
                active: false,
                ends_at: None,
                next_start: intervals
                    .iter()
                    .map(|(start, _)| *start)
                    .filter(|start| *start > now)
                    .min(),
            },
        }
    }

    /// The merged UTC intervals that can influence the local date of `now`.
    fn intervals_around(&self, now: Timestamp, tz: Tz) -> Vec<(Timestamp, Timestamp)> {
        let today = now.with_timezone(&tz).date_naive();
        let mut intervals = Vec::new();
        for offset in [-1_i64, 0] {
            let Some(date) = today
                .checked_sub_days(Days::new(offset.unsigned_abs()))
                .or_else(|| today.checked_add_days(Days::new(offset.unsigned_abs())))
            else {
                continue;
            };
            if !self.applies_on(date) {
                continue;
            }
            let Some(windows) = self.days.windows_for(date, &self.windows) else {
                continue;
            };
            for window in windows {
                let Some((start_local, end_local)) = window.interval_for(date) else {
                    continue;
                };
                let (Some(start), Some(end)) = (
                    resolve_date_time(start_local.date(), start_local.time(), tz),
                    resolve_date_time(end_local.date(), end_local.time(), tz),
                ) else {
                    continue;
                };
                // A window entirely inside a DST gap collapses to zero length
                // and does not run (spec §9).
                if end > start {
                    intervals.push((start, end));
                }
            }
        }
        merge_intervals(intervals)
    }

    fn hourly_status(&self, now: Timestamp, tz: Tz, pattern: HourlyPattern) -> ScheduleStatus {
        let local = now.with_timezone(&tz);
        let date = local.date_naive();
        if !self.applies_on(date) {
            return ScheduleStatus::inactive();
        }
        let hour_local = date
            .and_hms_opt(local.hour(), 0, 0)
            .unwrap_or(local.naive_local());
        let Some(hour_start) = resolve_date_time(hour_local.date(), hour_local.time(), tz) else {
            return ScheduleStatus::inactive();
        };
        let on_end =
            hour_start + TimeDelta::from_std(pattern.on).unwrap_or_else(|_| TimeDelta::hours(1));
        if now < on_end {
            ScheduleStatus {
                active: true,
                ends_at: Some(on_end),
                next_start: None,
            }
        } else {
            let next_local = hour_local + TimeDelta::hours(1);
            let next_start =
                resolve_date_time(next_local.date(), next_local.time(), tz).unwrap_or(on_end);
            ScheduleStatus {
                active: false,
                ends_at: None,
                next_start: Some(next_start),
            }
        }
    }

    /// The interval containing `now`, if the schedule is active.
    pub fn active_period(&self, now: Timestamp, tz: Tz) -> Option<Period> {
        let status = self.status(now, tz);
        status
            .active
            .then(|| Period::new(now, status.ends_at.unwrap_or(now)))
    }
}

/// Result of evaluating a schedule.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScheduleStatus {
    /// Whether rules apply right now.
    pub active: bool,
    /// When the current active run ends, if active.
    pub ends_at: Option<Timestamp>,
    /// When the next run starts, if inactive.
    pub next_start: Option<Timestamp>,
}

impl ScheduleStatus {
    fn inactive() -> Self {
        Self {
            active: false,
            ends_at: None,
            next_start: None,
        }
    }
}

/// Merge overlapping and adjacent intervals.
fn merge_intervals(mut intervals: Vec<(Timestamp, Timestamp)>) -> Vec<(Timestamp, Timestamp)> {
    intervals.sort_unstable();
    let mut merged: Vec<(Timestamp, Timestamp)> = Vec::with_capacity(intervals.len());
    for (start, end) in intervals {
        match merged.last_mut() {
            Some((_, current_end)) if start <= *current_end => {
                if end > *current_end {
                    *current_end = end;
                }
            }
            _ => merged.push((start, end)),
        }
    }
    merged
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};

    fn window(start: (u32, u32), end: (u32, u32)) -> Window {
        Window::new(
            NaiveTime::from_hms_opt(start.0, start.1, 0).unwrap(),
            NaiveTime::from_hms_opt(end.0, end.1, 0).unwrap(),
        )
    }

    fn utc(y: i32, m: u32, d: u32, h: u32, min: u32, s: u32) -> Timestamp {
        Utc.with_ymd_and_hms(y, m, d, h, min, s).unwrap()
    }

    // 2026-10-22 is a Thursday.

    #[test]
    fn window_boundaries_are_half_open() {
        let schedule = Schedule::daily(vec![window((9, 0), (17, 0))]).unwrap();
        assert!(
            !schedule
                .status(utc(2026, 10, 22, 8, 59, 59), chrono_tz::UTC)
                .active
        );
        assert!(
            schedule
                .status(utc(2026, 10, 22, 9, 0, 0), chrono_tz::UTC)
                .active
        );
        assert!(
            schedule
                .status(utc(2026, 10, 22, 16, 59, 59), chrono_tz::UTC)
                .active
        );
        assert!(
            !schedule
                .status(utc(2026, 10, 22, 17, 0, 0), chrono_tz::UTC)
                .active
        );
    }

    #[test]
    fn cross_midnight_window_belongs_to_start_day() {
        let schedule = Schedule::daily(vec![window((22, 0), (6, 0))]).unwrap();
        // Friday 2026-10-23 22:00 -> Saturday 02:00.
        assert!(
            schedule
                .status(utc(2026, 10, 24, 2, 0, 0), chrono_tz::UTC)
                .active
        );
        assert!(
            !schedule
                .status(utc(2026, 10, 24, 7, 0, 0), chrono_tz::UTC)
                .active
        );
    }

    #[test]
    fn weekday_schedule_skips_weekends() {
        let schedule =
            Schedule::days(DayPattern::weekdays(), vec![window((8, 0), (17, 0))]).unwrap();
        assert!(
            schedule
                .status(utc(2026, 10, 23, 9, 0, 0), chrono_tz::UTC)
                .active
        ); // Friday
        assert!(
            !schedule
                .status(utc(2026, 10, 24, 9, 0, 0), chrono_tz::UTC)
                .active
        ); // Saturday
        assert!(
            schedule
                .status(utc(2026, 10, 26, 9, 0, 0), chrono_tz::UTC)
                .active
        ); // Monday
    }

    #[test]
    fn overlapping_and_adjacent_windows_merge() {
        let schedule = Schedule::intervals(vec![
            window((9, 0), (12, 0)),
            window((11, 0), (13, 0)),
            window((13, 0), (14, 0)),
        ])
        .unwrap();
        let status = schedule.status(utc(2026, 10, 22, 10, 0, 0), chrono_tz::UTC);
        assert!(status.active);
        assert_eq!(status.ends_at, Some(utc(2026, 10, 22, 14, 0, 0)));
    }

    #[test]
    fn interval_gaps_allow() {
        let schedule =
            Schedule::intervals(vec![window((9, 0), (12, 0)), window((14, 0), (18, 0))]).unwrap();
        assert!(
            !schedule
                .status(utc(2026, 10, 22, 12, 30, 0), chrono_tz::UTC)
                .active
        );
        assert!(
            !schedule
                .status(utc(2026, 10, 22, 13, 59, 0), chrono_tz::UTC)
                .active
        );
        assert!(
            schedule
                .status(utc(2026, 10, 22, 14, 0, 0), chrono_tz::UTC)
                .active
        );
    }

    #[test]
    fn full_day_sunday_runs_all_day() {
        let schedule = Schedule::full_day(DayPattern::day(Weekday::Sun)).unwrap();
        assert!(
            schedule
                .status(utc(2026, 10, 25, 0, 0, 0), chrono_tz::UTC)
                .active
        );
        assert!(
            schedule
                .status(utc(2026, 10, 25, 23, 59, 59), chrono_tz::UTC)
                .active
        );
        assert!(
            !schedule
                .status(utc(2026, 10, 26, 0, 0, 0), chrono_tz::UTC)
                .active
        );
    }

    #[test]
    fn one_time_range_is_bounded() {
        let range = DateRange::new(
            NaiveDate::from_ymd_opt(2026, 11, 3).unwrap(),
            NaiveDate::from_ymd_opt(2026, 11, 3).unwrap(),
        )
        .unwrap();
        let schedule = Schedule::one_time(range, vec![window((9, 0), (17, 0))]).unwrap();
        assert!(
            !schedule
                .status(utc(2026, 11, 2, 10, 0, 0), chrono_tz::UTC)
                .active
        );
        assert!(
            schedule
                .status(utc(2026, 11, 3, 10, 0, 0), chrono_tz::UTC)
                .active
        );
        assert!(
            !schedule
                .status(utc(2026, 11, 4, 10, 0, 0), chrono_tz::UTC)
                .active
        );
    }

    #[test]
    fn hourly_pattern_blocks_the_off_minutes() {
        let schedule = Schedule::hourly(DayPattern::EveryDay, HourlyPattern::fifty_ten()).unwrap();
        assert!(
            schedule
                .status(utc(2026, 10, 22, 9, 0, 0), chrono_tz::UTC)
                .active
        );
        assert!(
            schedule
                .status(utc(2026, 10, 22, 9, 49, 59), chrono_tz::UTC)
                .active
        );
        assert!(
            !schedule
                .status(utc(2026, 10, 22, 9, 50, 0), chrono_tz::UTC)
                .active
        );
        assert!(
            schedule
                .status(utc(2026, 10, 22, 10, 0, 0), chrono_tz::UTC)
                .active
        );
    }

    #[test]
    fn dst_gap_window_does_not_run() {
        use chrono_tz::America::New_York;
        let schedule = Schedule::daily(vec![window((2, 0), (3, 0))]).unwrap();
        // 2026-03-08 02:00-03:00 does not exist in New York.
        let noon = resolve_date_time(
            NaiveDate::from_ymd_opt(2026, 3, 8).unwrap(),
            NaiveTime::from_hms_opt(12, 0, 0).unwrap(),
            New_York,
        )
        .unwrap();
        assert!(!schedule.status(noon, New_York).active);
        // The same wall clock on a normal day is active.
        let normal = resolve_date_time(
            NaiveDate::from_ymd_opt(2026, 3, 9).unwrap(),
            NaiveTime::from_hms_opt(2, 30, 0).unwrap(),
            New_York,
        )
        .unwrap();
        assert!(schedule.status(normal, New_York).active);
    }

    #[test]
    fn invalid_schedules_are_rejected() {
        assert!(Schedule::daily(vec![]).is_err());
        assert!(Schedule::daily(vec![window((9, 0), (9, 0))]).is_err());
        assert!(
            Schedule::daily(vec![Window::new(
                NaiveTime::from_hms_opt(9, 0, 30).unwrap(),
                NaiveTime::from_hms_opt(17, 0, 0).unwrap()
            )])
            .is_err()
        );
        assert!(HourlyPattern::new(Duration::from_secs(60), Duration::from_secs(60)).is_err());
    }
}
