//! Usage events and counters.
//!
//! Events are the source of truth; counters are queries over them. A foreground
//! interval is closed by the next foreground change, screen lock, suspend, or
//! daemon shutdown. On crash recovery the daemon closes the open interval at
//! the last heartbeat, so usage is under-counted by at most the heartbeat
//! interval and never over-counted (`docs/spec/blocking-engine.md` §4, §7).

use std::collections::BTreeMap;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::app::{AppKey, Host, WindowKey};
use crate::domain::Target;
use crate::period::Period;
use crate::reason::ReasonCode;
use crate::time::Timestamp;

/// The anti-thrash window for launch counting.
///
/// Switching away and back within this window is not a new launch. The value is
/// a StayProductive decision, not a measured reference value
/// (`docs/spec/blocking-engine.md` §4).
pub const LAUNCH_ANTI_THRASH: Duration = Duration::from_secs(5);

/// How many usage events the in-memory log keeps.
///
/// The log is a debugging and future-statistics aid; the ledger's intervals,
/// launches, pickups and blocks are the data that must survive a restart, so
/// the event tail is bounded and never persisted.
pub const EVENT_LOG_CAP: usize = 1024;

/// A category of usage event.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum UsageKind {
    /// An app became foreground.
    Foreground,
    /// An app left the foreground.
    Background,
    /// A launch was counted (after anti-thrash filtering).
    Launch,
    /// The screen was unlocked.
    Pickup,
    /// A browser reported a site visit.
    SiteVisit,
    /// A decision blocked the target.
    Blocked,
    /// A profile was paused.
    Paused,
    /// A profile was resumed.
    Resumed,
    /// The session locked.
    ScreenLocked,
    /// The session unlocked.
    ScreenUnlocked,
    /// The session suspended.
    Suspended,
    /// The session resumed.
    ResumedFromSuspend,
}

/// Where an observation came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum EventSource {
    /// X11 EWMH observer.
    X11,
    /// sway/i3 IPC observer.
    Sway,
    /// Hyprland IPC observer.
    Hyprland,
    /// GNOME Shell extension.
    Gnome,
    /// KWin script.
    KWin,
    /// Browser extension.
    Extension,
    /// CLI, tray, or UI action.
    Manual,
    /// Internal timer wheel.
    Timer,
    /// Restored from storage.
    Restore,
}

/// An append-only usage event.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UsageEvent {
    /// Monotonic event sequence within this database.
    pub id: u64,
    /// UTC instant.
    pub at: Timestamp,
    /// What happened.
    pub kind: UsageKind,
    /// Application, when applicable.
    pub app: Option<AppKey>,
    /// Website host, when applicable.
    pub site: Option<Host>,
    /// Ephemeral window key; never exported by default.
    pub window: Option<WindowKey>,
    /// Reason for Blocked/Paused events.
    pub reason: Option<ReasonCode>,
    /// Observer that reported the event.
    pub source: EventSource,
}

/// A closed foreground interval.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UsageInterval {
    /// Application.
    pub app: AppKey,
    /// Inclusive start.
    pub start: Timestamp,
    /// Exclusive end.
    pub end: Timestamp,
}

impl UsageInterval {
    /// Clip the interval to a period, returning its duration.
    pub fn clipped_duration(&self, period: &Period) -> Duration {
        period
            .clip(self.start, self.end)
            .map(|(start, end)| duration_between(start, end))
            .unwrap_or_default()
    }
}

/// A counted launch.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LaunchRecord {
    /// Application launched.
    pub app: AppKey,
    /// When the launch was counted.
    pub at: Timestamp,
}

/// A block that was enforced, for history and statistics.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlockRecord {
    /// When the block was enforced.
    pub at: Timestamp,
    /// What was blocked.
    pub target: Target,
    /// Why.
    pub reason: ReasonCode,
    /// Known end, if any.
    pub until: Option<Timestamp>,
    /// Profile that produced the rule, if any.
    pub profile: Option<crate::ids::ProfileId>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct OpenForeground {
    app: AppKey,
    start: Timestamp,
}

/// The in-memory usage ledger.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UsageLedger {
    intervals: Vec<UsageInterval>,
    open: Option<OpenForeground>,
    launches: Vec<LaunchRecord>,
    pickups: Vec<Timestamp>,
    blocks: Vec<BlockRecord>,
    #[serde(skip)]
    events: Vec<UsageEvent>,
    last_exit: BTreeMap<AppKey, Timestamp>,
    last_heartbeat: Option<Timestamp>,
    #[serde(skip)]
    next_event_id: u64,
}

impl UsageLedger {
    /// An empty ledger.
    pub fn new() -> Self {
        Self::default()
    }

    /// All closed intervals.
    pub fn intervals(&self) -> &[UsageInterval] {
        &self.intervals
    }

    /// All counted launches.
    pub fn launches(&self) -> &[LaunchRecord] {
        &self.launches
    }

    /// All pickups.
    pub fn pickups(&self) -> &[Timestamp] {
        &self.pickups
    }

    /// All enforced blocks.
    pub fn blocks(&self) -> &[BlockRecord] {
        &self.blocks
    }

    /// The append-only event log.
    pub fn events(&self) -> &[UsageEvent] {
        &self.events
    }

    /// Close a foreground interval left open by a crash.
    ///
    /// Called once after restoring state from storage, before the daemon
    /// publishes it. The interval ends at the last heartbeat.
    pub fn recover(&mut self) {
        if let Some(open) = self.open.take() {
            let end = self.last_heartbeat.unwrap_or(open.start);
            if end > open.start {
                self.intervals.push(UsageInterval {
                    app: open.app.clone(),
                    start: open.start,
                    end,
                });
                self.last_exit.insert(open.app, end);
            }
        }
    }

    /// The foreground app, if tracking.
    pub fn current_app(&self) -> Option<&AppKey> {
        self.open.as_ref().map(|open| &open.app)
    }

    /// When the current foreground app was first observed.
    pub fn current_since(&self) -> Option<Timestamp> {
        self.open.as_ref().map(|open| open.start)
    }

    /// The last heartbeat, used to close a crashed interval.
    pub fn last_heartbeat(&self) -> Option<Timestamp> {
        self.last_heartbeat
    }

    /// Update the heartbeat without changing attribution.
    pub fn heartbeat(&mut self, at: Timestamp) {
        self.last_heartbeat = Some(at);
    }

    /// Record a foreground change.
    pub fn foreground(&mut self, app: AppKey, at: Timestamp) {
        if let Some(open) = &self.open {
            if open.app == app {
                self.heartbeat(at);
                return;
            }
        }
        self.close_foreground(at);
        let last_exit = self.last_exit.get(&app).copied();
        let is_launch = last_exit.is_none_or(|exit| {
            at - exit > chrono::TimeDelta::from_std(LAUNCH_ANTI_THRASH).unwrap_or_default()
        });
        self.push_event(UsageEvent {
            id: 0,
            at,
            kind: UsageKind::Foreground,
            app: Some(app.clone()),
            site: None,
            window: None,
            reason: None,
            source: EventSource::Restore,
        });
        if is_launch {
            self.launches.push(LaunchRecord {
                app: app.clone(),
                at,
            });
            self.push_event(UsageEvent {
                id: 0,
                at,
                kind: UsageKind::Launch,
                app: Some(app.clone()),
                site: None,
                window: None,
                reason: None,
                source: EventSource::Restore,
            });
        }
        self.open = Some(OpenForeground { app, start: at });
        self.heartbeat(at);
    }

    /// Close the current foreground interval at `at`.
    pub fn close_foreground(&mut self, at: Timestamp) {
        if let Some(open) = self.open.take() {
            if at > open.start {
                self.intervals.push(UsageInterval {
                    app: open.app.clone(),
                    start: open.start,
                    end: at,
                });
                self.last_exit.insert(open.app.clone(), at);
                self.push_event(UsageEvent {
                    id: 0,
                    at,
                    kind: UsageKind::Background,
                    app: Some(open.app),
                    site: None,
                    window: None,
                    reason: None,
                    source: EventSource::Restore,
                });
            }
        }
        self.heartbeat(at);
    }

    /// Close a restored interval at the last heartbeat.
    ///
    /// The daemon persists closed intervals; if it crashed while an app was
    /// foreground, the open tail is represented by a synthetic interval ending
    /// at the last heartbeat.
    pub fn close_crashed_interval(&mut self, app: AppKey, start: Timestamp, heartbeat: Timestamp) {
        if heartbeat > start {
            self.intervals.push(UsageInterval {
                app: app.clone(),
                start,
                end: heartbeat,
            });
            self.last_exit.insert(app, heartbeat);
        }
        self.last_heartbeat = Some(heartbeat);
    }

    /// Record a screen lock and stop tracking.
    pub fn screen_locked(&mut self, at: Timestamp) {
        self.close_foreground(at);
        self.push_event(UsageEvent {
            id: 0,
            at,
            kind: UsageKind::ScreenLocked,
            app: None,
            site: None,
            window: None,
            reason: None,
            source: EventSource::Restore,
        });
    }

    /// Record a screen unlock as a pickup.
    pub fn screen_unlocked(&mut self, at: Timestamp) {
        self.pickups.push(at);
        self.push_event(UsageEvent {
            id: 0,
            at,
            kind: UsageKind::Pickup,
            app: None,
            site: None,
            window: None,
            reason: None,
            source: EventSource::Restore,
        });
        self.heartbeat(at);
    }

    /// Record suspend, closing the foreground interval.
    pub fn suspended(&mut self, at: Timestamp) {
        self.close_foreground(at);
        self.push_event(UsageEvent {
            id: 0,
            at,
            kind: UsageKind::Suspended,
            app: None,
            site: None,
            window: None,
            reason: None,
            source: EventSource::Restore,
        });
    }

    /// Record resume. The compositor reports the foreground app separately.
    pub fn resumed(&mut self, at: Timestamp) {
        self.push_event(UsageEvent {
            id: 0,
            at,
            kind: UsageKind::ResumedFromSuspend,
            app: None,
            site: None,
            window: None,
            reason: None,
            source: EventSource::Restore,
        });
        self.heartbeat(at);
    }

    /// Record a browser site visit.
    pub fn site_visit(&mut self, app: Option<AppKey>, site: Host, at: Timestamp) {
        self.push_event(UsageEvent {
            id: 0,
            at,
            kind: UsageKind::SiteVisit,
            app,
            site: Some(site),
            window: None,
            reason: None,
            source: EventSource::Extension,
        });
        self.heartbeat(at);
    }

    /// Record an enforced block.
    pub fn blocked(
        &mut self,
        target: Target,
        reason: ReasonCode,
        at: Timestamp,
        until: Option<Timestamp>,
        profile: Option<crate::ids::ProfileId>,
    ) {
        self.blocks.push(BlockRecord {
            at,
            target: target.clone(),
            reason,
            until,
            profile,
        });
        self.push_event(UsageEvent {
            id: 0,
            at,
            kind: UsageKind::Blocked,
            app: match &target {
                Target::App(app) => Some(app.clone()),
                Target::Site(_) => None,
            },
            site: match &target {
                Target::Site(host) => Some(host.clone()),
                Target::App(_) => None,
            },
            window: None,
            reason: Some(reason),
            source: EventSource::Restore,
        });
    }

    /// Record pause/resume for statistics.
    pub fn paused(&mut self, at: Timestamp) {
        self.push_event(UsageEvent {
            id: 0,
            at,
            kind: UsageKind::Paused,
            app: None,
            site: None,
            window: None,
            reason: None,
            source: EventSource::Manual,
        });
    }

    /// Record resume from pause.
    pub fn resumed_from_pause(&mut self, at: Timestamp) {
        self.push_event(UsageEvent {
            id: 0,
            at,
            kind: UsageKind::Resumed,
            app: None,
            site: None,
            window: None,
            reason: None,
            source: EventSource::Manual,
        });
    }

    /// Total foreground time attributed to `app` inside `period`.
    ///
    /// Includes the currently open interval up to `now`, so a decision at a
    /// limit boundary does not lag behind the last foreground change.
    pub fn usage(&self, app: &AppKey, period: &Period, now: Timestamp) -> Duration {
        let closed: Duration = self
            .intervals
            .iter()
            .filter(|interval| &interval.app == app)
            .map(|interval| interval.clipped_duration(period))
            .sum();
        let open = self
            .open
            .as_ref()
            .filter(|open| &open.app == app)
            .and_then(|open| {
                let end = now.min(period.end);
                (open.start < end).then(|| duration_between(open.start.max(period.start), end))
            })
            .unwrap_or_default();
        closed + open
    }

    /// Foreground time per app inside `period`, including the open interval.
    ///
    /// Apps with no time inside the period are absent from the map.
    pub fn usage_per_app(&self, period: &Period, now: Timestamp) -> BTreeMap<AppKey, Duration> {
        let mut totals: BTreeMap<AppKey, Duration> = BTreeMap::new();
        for interval in &self.intervals {
            let clipped = interval.clipped_duration(period);
            if clipped > Duration::ZERO {
                *totals.entry(interval.app.clone()).or_default() += clipped;
            }
        }
        if let Some(open) = &self.open {
            let end = now.min(period.end);
            if open.start < end {
                let start = open.start.max(period.start);
                *totals.entry(open.app.clone()).or_default() += duration_between(start, end);
            }
        }
        totals
    }

    /// Number of counted launches of `app` inside `period`.
    pub fn launch_count(&self, app: &AppKey, period: &Period) -> u32 {
        u32::try_from(
            self.launches
                .iter()
                .filter(|launch| &launch.app == app && period.contains(launch.at))
                .count(),
        )
        .unwrap_or(u32::MAX)
    }

    /// Number of pickups inside `period`.
    pub fn pickup_count(&self, period: &Period) -> u32 {
        u32::try_from(
            self.pickups
                .iter()
                .filter(|at| period.contains(**at))
                .count(),
        )
        .unwrap_or(u32::MAX)
    }

    /// Number of counted launches inside `period`, across every app.
    pub fn launch_total(&self, period: &Period) -> u32 {
        u32::try_from(
            self.launches
                .iter()
                .filter(|launch| period.contains(launch.at))
                .count(),
        )
        .unwrap_or(u32::MAX)
    }

    /// Number of enforced blocks recorded inside `period`.
    pub fn block_count(&self, period: &Period) -> u32 {
        u32::try_from(
            self.blocks
                .iter()
                .filter(|block| period.contains(block.at))
                .count(),
        )
        .unwrap_or(u32::MAX)
    }

    /// Foreground time per app, longest first, ties broken by app key.
    ///
    /// Stable ordering keeps the Stats ranking from jumping between polls when
    /// two apps have the same duration.
    pub fn usage_ranking(&self, period: &Period, now: Timestamp) -> Vec<(AppKey, Duration)> {
        let mut rows: Vec<(AppKey, Duration)> = self
            .usage_per_app(period, now)
            .into_iter()
            .filter(|(_app, duration)| *duration > Duration::ZERO)
            .collect();
        rows.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        rows
    }

    /// Total foreground time attributed to any app inside `period`.
    pub fn total_usage(&self, period: &Period, now: Timestamp) -> Duration {
        self.usage_per_app(period, now).values().copied().sum()
    }

    /// The instant at which cumulative usage of `app` inside `period` reached
    /// `threshold`, if it did.
    ///
    /// Walked chronologically so wait timers anchor to the first crossing.
    pub fn usage_crossed_at(
        &self,
        app: &AppKey,
        period: &Period,
        threshold: Duration,
        now: Timestamp,
    ) -> Option<Timestamp> {
        let target = chrono::TimeDelta::from_std(threshold).ok()?;
        let mut accumulated = chrono::TimeDelta::zero();
        let mut intervals: Vec<(Timestamp, Timestamp)> = self
            .intervals
            .iter()
            .filter(|interval| &interval.app == app)
            .filter_map(|interval| period.clip(interval.start, interval.end))
            .collect();
        if let Some(open) = self.open.as_ref().filter(|open| &open.app == app) {
            let end = now.min(period.end);
            if let Some(clip) = period.clip(open.start, end) {
                intervals.push(clip);
            }
        }
        intervals.sort_unstable_by_key(|(start, _)| *start);
        for (start, end) in intervals {
            let length = end - start;
            if accumulated + length >= target {
                return Some(start + (target - accumulated));
            }
            accumulated += length;
        }
        None
    }

    /// Restore a ledger from persisted pieces.
    pub fn restore(
        intervals: Vec<UsageInterval>,
        launches: Vec<LaunchRecord>,
        pickups: Vec<Timestamp>,
        blocks: Vec<BlockRecord>,
    ) -> Self {
        let mut last_exit = BTreeMap::new();
        for interval in &intervals {
            last_exit
                .entry(interval.app.clone())
                .and_modify(|at: &mut Timestamp| *at = (*at).max(interval.end))
                .or_insert(interval.end);
        }
        let last_heartbeat = intervals.iter().map(|interval| interval.end).max();
        Self {
            intervals,
            open: None,
            launches,
            pickups,
            blocks,
            events: Vec::new(),
            last_exit,
            last_heartbeat,
            next_event_id: 0,
        }
    }

    fn push_event(&mut self, mut event: UsageEvent) {
        event.id = self.next_event_id;
        self.next_event_id += 1;
        self.events.push(event);
        let excess = self.events.len().saturating_sub(EVENT_LOG_CAP);
        if excess > 0 {
            self.events.drain(..excess);
        }
    }
}

fn duration_between(start: Timestamp, end: Timestamp) -> Duration {
    let delta = end - start;
    if delta <= chrono::TimeDelta::zero() {
        return Duration::ZERO;
    }
    delta.to_std().unwrap_or(Duration::ZERO)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};

    fn ts(minutes: i64) -> Timestamp {
        Utc.with_ymd_and_hms(2026, 10, 22, 9, 0, 0).unwrap() + chrono::TimeDelta::minutes(minutes)
    }

    fn ts_secs(seconds: i64) -> Timestamp {
        Utc.with_ymd_and_hms(2026, 10, 22, 9, 0, 0).unwrap() + chrono::TimeDelta::seconds(seconds)
    }

    fn period() -> Period {
        Period::new(ts(0), ts(60))
    }

    #[test]
    fn usage_is_attributed_to_exactly_one_app() {
        let mut ledger = UsageLedger::new();
        let firefox = AppKey::new("firefox").unwrap();
        let editor = AppKey::new("editor").unwrap();
        ledger.foreground(firefox.clone(), ts(0));
        ledger.foreground(editor.clone(), ts(10));
        ledger.foreground(firefox.clone(), ts(20));
        ledger.close_foreground(ts(30));
        assert_eq!(
            ledger.usage(&firefox, &period(), ts(30)),
            Duration::from_secs(20 * 60)
        );
        assert_eq!(
            ledger.usage(&editor, &period(), ts(30)),
            Duration::from_secs(10 * 60)
        );
    }

    #[test]
    fn open_interval_counts_up_to_now() {
        let mut ledger = UsageLedger::new();
        let firefox = AppKey::new("firefox").unwrap();
        ledger.foreground(firefox.clone(), ts(0));
        assert_eq!(
            ledger.usage(&firefox, &period(), ts(7)),
            Duration::from_secs(7 * 60)
        );
    }

    #[test]
    fn launches_ignore_quick_switches() {
        let mut ledger = UsageLedger::new();
        let firefox = AppKey::new("firefox").unwrap();
        let editor = AppKey::new("editor").unwrap();
        ledger.foreground(firefox.clone(), ts_secs(0));
        ledger.foreground(editor.clone(), ts_secs(1));
        ledger.foreground(firefox.clone(), ts_secs(2)); // within 5 s, not a launch
        ledger.foreground(editor.clone(), ts_secs(3));
        ledger.foreground(firefox.clone(), ts_secs(10)); // after 5 s, counted
        assert_eq!(ledger.launch_count(&firefox, &period()), 2);
    }

    #[test]
    fn pickups_count_once_per_unlock() {
        let mut ledger = UsageLedger::new();
        ledger.screen_unlocked(ts(1));
        ledger.screen_unlocked(ts(2));
        assert_eq!(ledger.pickup_count(&period()), 2);
    }

    #[test]
    fn ranking_is_by_time_then_app_key() {
        let mut ledger = UsageLedger::new();
        let beta = AppKey::new("beta").unwrap();
        let alpha = AppKey::new("alpha").unwrap();
        let gamma = AppKey::new("gamma").unwrap();
        ledger.foreground(beta.clone(), ts(0));
        ledger.foreground(alpha.clone(), ts(10));
        ledger.foreground(gamma.clone(), ts(20));
        ledger.close_foreground(ts(40));
        // beta 10 min, alpha 10 min, gamma 20 min.
        let ranking = ledger.usage_ranking(&period(), ts(40));
        assert_eq!(
            ranking,
            vec![
                (gamma, Duration::from_secs(20 * 60)),
                (alpha, Duration::from_secs(10 * 60)),
                (beta, Duration::from_secs(10 * 60)),
            ]
        );
        assert_eq!(
            ledger.total_usage(&period(), ts(40)),
            Duration::from_secs(40 * 60)
        );
    }

    #[test]
    fn totals_count_launches_and_blocks_once() {
        let mut ledger = UsageLedger::new();
        let app = AppKey::new("social").unwrap();
        ledger.foreground(app.clone(), ts(0));
        ledger.close_foreground(ts(10));
        ledger.foreground(app.clone(), ts(20));
        ledger.blocked(
            crate::domain::Target::App(app.clone()),
            crate::reason::ReasonCode::LimitDaily,
            ts(30),
            None,
            None,
        );
        assert_eq!(ledger.launch_total(&period()), 2);
        assert_eq!(ledger.block_count(&period()), 1);
    }

    #[test]
    fn usage_crossing_is_anchored_to_the_first_crossing() {
        let mut ledger = UsageLedger::new();
        let app = AppKey::new("social").unwrap();
        ledger.foreground(app.clone(), ts(0));
        ledger.close_foreground(ts(10));
        ledger.foreground(app.clone(), ts(20));
        // 10 minutes done; 5 more minutes of usage reaches 15 minutes, at t=25.
        assert_eq!(
            ledger.usage_crossed_at(&app, &period(), Duration::from_secs(15 * 60), ts(40)),
            Some(ts(25))
        );
    }

    #[test]
    fn crash_recovery_closes_at_last_heartbeat() {
        let mut ledger = UsageLedger::new();
        let app = AppKey::new("editor").unwrap();
        ledger.close_crashed_interval(app.clone(), ts(0), ts(5));
        assert_eq!(
            ledger.usage(&app, &period(), ts(30)),
            Duration::from_secs(5 * 60)
        );
    }

    #[test]
    fn event_log_is_bounded_and_not_persisted() {
        let mut ledger = UsageLedger::new();
        let a = AppKey::new("alpha").unwrap();
        let b = AppKey::new("beta").unwrap();
        for i in 0..(EVENT_LOG_CAP + 100) {
            if i % 2 == 0 {
                ledger.foreground(a.clone(), ts_secs(i as i64));
            } else {
                ledger.foreground(b.clone(), ts_secs(i as i64));
            }
        }
        assert!(
            ledger.events().len() <= EVENT_LOG_CAP,
            "event log must stay bounded, got {}",
            ledger.events().len()
        );
        let encoded = serde_json::to_value(&ledger).unwrap();
        assert!(
            encoded.get("events").is_none(),
            "events must not be persisted with the state"
        );
        let restored: UsageLedger = serde_json::from_value(encoded).unwrap();
        assert!(restored.events().is_empty());
        assert_eq!(restored.intervals().len(), ledger.intervals().len());
        assert_eq!(restored.last_heartbeat(), ledger.last_heartbeat());
    }

    #[test]
    fn usage_per_app_sums_closed_and_open_intervals() {
        let mut ledger = UsageLedger::new();
        let firefox = AppKey::new("firefox").unwrap();
        let editor = AppKey::new("editor").unwrap();
        ledger.foreground(firefox.clone(), ts(0));
        ledger.foreground(editor.clone(), ts(10));
        ledger.foreground(firefox.clone(), ts(20));
        let totals = ledger.usage_per_app(&period(), ts(30));
        assert_eq!(
            totals.get(&firefox),
            Some(&Duration::from_secs(20 * 60)),
            "firefox: 10 closed minutes plus the open interval"
        );
        assert_eq!(totals.get(&editor), Some(&Duration::from_secs(10 * 60)));
        assert_eq!(totals.len(), 2, "no other app is attributed");
    }
}
