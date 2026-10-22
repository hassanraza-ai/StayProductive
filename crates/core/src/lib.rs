//! The StayProductive core.
//!
//! This crate owns everything that decides *whether* something is blocked. It
//! has no UI, no D-Bus, no compositor and no wall-clock reads: callers pass the
//! current instant and the observed event, and receive a [`engine::Decision`].
//!
//! See `docs/spec/domain-model.md`, `docs/spec/schedules-and-conditions.md` and
//! `docs/spec/blocking-engine.md` for the normative behaviour.

pub mod app;
pub mod domain;
pub mod engine;
pub mod error;
pub mod ids;
pub mod period;
pub mod reason;
pub mod schedule;
pub mod time;
pub mod usage;

pub use app::{AppKey, Host, HostPattern, Keyword, KeywordScope, SurfaceId, WindowKey};
pub use domain::{
    Activation, AppRule, AppRuleEntry, Commitment, CommitmentKind, CommitmentState, CountLimit,
    DeactivationMethod, LimitPeriod, PauseReason, PauseState, Profile, QuickBlock, Restriction,
    RuleSet, SiteAction, SiteRule, Target,
};
pub use engine::{Decision, Event, MessageKey, State, Verdict, decide};
pub use error::{Error, Result};
pub use ids::{CommitmentId, ProfileId, RuleId, SessionId, Ulid};
pub use period::{ClockSettings, Period, day_period, hour_period, logical_date};
pub use reason::ReasonCode;
pub use schedule::{
    DateRange, DayPattern, HourlyPattern, Schedule, ScheduleKind, ScheduleStatus, Window,
};
pub use time::Timestamp;
pub use usage::{
    BlockRecord, EventSource, LaunchRecord, UsageEvent, UsageInterval, UsageKind, UsageLedger,
};
