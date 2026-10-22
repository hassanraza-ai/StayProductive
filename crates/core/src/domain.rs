//! Domain entities: profiles, rules, schedules, pauses and commitments.
//!
//! The model follows `docs/spec/domain-model.md`. Runtime state (manual
//! activation, quick blocks, usage) lives in [`crate::engine::State`]; this
//! module holds the durable, serializable shape.

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use chrono::NaiveTime;
use serde::{Deserialize, Serialize};

use crate::app::{AppKey, Host, HostPattern, Keyword, KeywordScope};
use crate::error::{Error, Result};
use crate::ids::{CommitmentId, ProfileId, RuleId, Ulid};
use crate::reason::ReasonCode;
use crate::schedule::Schedule;
use crate::time::Timestamp;

/// A profile colour.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rgb {
    /// Red channel.
    pub r: u8,
    /// Green channel.
    pub g: u8,
    /// Blue channel.
    pub b: u8,
}

/// A reference to an icon theme name or bundled icon.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct IconRef(pub String);

/// A bundled template identifier.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TemplateId(pub String);

/// How a profile becomes active.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Activation {
    /// Only while manually activated. At most one manual profile is active.
    #[default]
    Manual,
    /// Whenever its schedule says so.
    Scheduled,
    /// Until manually disabled.
    Always,
}

/// Why a profile is paused.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum PauseReason {
    /// The user paused it.
    #[default]
    User,
    /// A break applies.
    Break,
    /// A strict cool-down is running.
    StrictCooldown,
    /// The system suspended tracking.
    System,
}

/// Persisted pause state.
///
/// Wall-clock anchors are used here; the daemon additionally records monotonic
/// anchors for breaks and cool-downs so suspend cannot consume them
/// (`docs/spec/domain-model.md` §9).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PauseState {
    /// Whether pause was requested.
    pub paused: bool,
    /// When the pause ends; `None` while paused means indefinite.
    pub paused_until: Option<Timestamp>,
    /// Why the profile is paused.
    pub reason: PauseReason,
}

impl PauseState {
    /// A running pause.
    pub fn until(at: Timestamp, reason: PauseReason) -> Self {
        Self {
            paused: true,
            paused_until: Some(at),
            reason,
        }
    }

    /// An indefinite pause.
    pub fn indefinite(reason: PauseReason) -> Self {
        Self {
            paused: true,
            paused_until: None,
            reason,
        }
    }

    /// Whether the pause suppresses rules at `now`.
    pub fn is_paused(&self, now: Timestamp) -> bool {
        self.paused && self.paused_until.is_none_or(|until| now < until)
    }

    /// Clear the pause.
    pub fn resume(&mut self) {
        self.paused = false;
        self.paused_until = None;
        self.reason = PauseReason::User;
    }

    /// End an expired pause, returning whether it was cleared.
    pub fn expire(&mut self, now: Timestamp) -> bool {
        if self.paused && self.paused_until.is_some_and(|until| now >= until) {
            self.resume();
            true
        } else {
            false
        }
    }
}

/// Daily or hourly (used by count limits).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum LimitPeriod {
    /// The logical day, honouring start-of-day.
    Daily,
    /// The local wall-clock hour.
    Hourly,
}

/// A count limit such as launches or pickups.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CountLimit {
    /// Maximum count inside the period. The enum is exhausted at the limit.
    pub count: u32,
    /// Period the count resets in.
    pub per: LimitPeriod,
}

impl CountLimit {
    /// Create a count limit. Zero is invalid.
    pub fn new(count: u32, per: LimitPeriod) -> Result<Self> {
        if count == 0 {
            return Err(Error::InvalidRule("count limit must be positive".into()));
        }
        Ok(Self { count, per })
    }

    /// The reason used when the limit is exhausted.
    pub const fn reason(self) -> ReasonCode {
        match self.per {
            LimitPeriod::Daily | LimitPeriod::Hourly => ReasonCode::Launch,
        }
    }
}

/// What a profile does to an application.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AppRule {
    /// Always block while the profile is active.
    Block,
    /// Daily foreground allowance.
    DailyLimit {
        /// Maximum foreground time in the logical day.
        limit: Duration,
    },
    /// Hourly foreground allowance.
    HourlyLimit {
        /// Maximum foreground time in the local hour.
        limit: Duration,
    },
    /// Maximum number of launches per period.
    LaunchLimit {
        /// Maximum launches.
        count: u32,
        /// Reset period.
        per: LimitPeriod,
    },
    /// Deny access for a waiting period after sustained use.
    WaitTimer {
        /// Usage that triggers the wait.
        use_for: Duration,
        /// How long access is denied.
        wait: Duration,
        /// Halve the remaining wait once if the app was not used in the first
        /// half of the wait.
        adaptive: bool,
    },
    /// Earn access to this app by using another app.
    Goal {
        /// App whose usage earns credit.
        earn_for: AppKey,
        /// Credit required per grant.
        required: Duration,
        /// Access granted when `required` is earned.
        grant: Duration,
        /// Credit already earned, consumed while this app is used.
        earned: Duration,
    },
}

impl AppRule {
    /// Whether the rule can ever block.
    pub const fn is_blocking(&self) -> bool {
        true
    }

    /// Validation-independent priority when merging duplicate rules.
    fn rank(&self) -> u8 {
        match self {
            Self::Block => 0,
            Self::DailyLimit { .. } => 1,
            Self::HourlyLimit { .. } => 2,
            Self::LaunchLimit { .. } => 3,
            Self::WaitTimer { .. } => 4,
            Self::Goal { .. } => 5,
        }
    }
}

fn merge_duration_min(a: Duration, b: Duration) -> Duration {
    a.min(b)
}

fn merge_app_rule(current: &AppRule, incoming: &AppRule) -> AppRule {
    use AppRule::*;
    if current.rank() != incoming.rank() {
        return if incoming.rank() < current.rank() {
            incoming.clone()
        } else {
            current.clone()
        };
    }
    match (current, incoming) {
        (Block, Block) => Block,
        (DailyLimit { limit: a }, DailyLimit { limit: b }) => DailyLimit {
            limit: merge_duration_min(*a, *b),
        },
        (HourlyLimit { limit: a }, HourlyLimit { limit: b }) => HourlyLimit {
            limit: merge_duration_min(*a, *b),
        },
        (LaunchLimit { count: a, per: pa }, LaunchLimit { count: b, per: pb }) => LaunchLimit {
            count: (*a).min(*b),
            per: match (pa, pb) {
                (LimitPeriod::Hourly, _) | (_, LimitPeriod::Hourly) => LimitPeriod::Hourly,
                _ => LimitPeriod::Daily,
            },
        },
        (
            WaitTimer {
                use_for: ua,
                wait: wa,
                adaptive: aa,
            },
            WaitTimer {
                use_for: ub,
                wait: wb,
                adaptive: ab,
            },
        ) => WaitTimer {
            use_for: merge_duration_min(*ua, *ub),
            wait: (*wa).max(*wb),
            adaptive: *aa || *ab,
        },
        _ => current.clone(),
    }
}

/// An app rule with stable identity.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppRuleEntry {
    /// Stable rule identifier.
    pub id: RuleId,
    /// The rule itself.
    pub rule: AppRule,
}

/// What a site rule does.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SiteAction {
    /// Block navigation.
    Block,
    /// Apply a daily time allowance to the site.
    Limit(Duration),
    /// Include the domain in adult filtering.
    Adult,
}

/// A site rule.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SiteRule {
    /// Stable rule identifier.
    pub id: RuleId,
    /// Host pattern.
    pub pattern: HostPattern,
    /// Action while the profile is active.
    pub action: SiteAction,
}

/// A keyword rule.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeywordRule {
    /// Stable rule identifier.
    pub id: RuleId,
    /// Normalized keyword.
    pub text: Keyword,
    /// Where the keyword is matched.
    pub scope: KeywordScope,
}

/// A profile's complete rule set.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuleSet {
    /// App rules by stable key.
    pub apps: BTreeMap<AppKey, AppRuleEntry>,
    /// Site rules, evaluated in order.
    pub sites: Vec<SiteRule>,
    /// Keyword rules.
    pub keywords: Vec<KeywordRule>,
    /// Apps that override non-strict app rules.
    pub allow_apps: BTreeSet<AppKey>,
    /// Hosts that override non-strict site rules.
    pub allow_sites: Vec<HostPattern>,
    /// Global pickup (unlock) count limit.
    pub pickup_limit: Option<CountLimit>,
}

impl RuleSet {
    /// Whether the set contains no rules.
    pub fn is_empty(&self) -> bool {
        self.apps.is_empty()
            && self.sites.is_empty()
            && self.keywords.is_empty()
            && self.allow_apps.is_empty()
            && self.allow_sites.is_empty()
            && self.pickup_limit.is_none()
    }

    /// Add or merge an app rule. Duplicate rules keep the stricter outcome and
    /// the original identity.
    pub fn set_app_rule(&mut self, app: AppKey, rule: AppRule) -> RuleId {
        if let Some(entry) = self.apps.get_mut(&app) {
            entry.rule = merge_app_rule(&entry.rule, &rule);
            entry.id
        } else {
            let id = Ulid::new();
            self.apps.insert(app, AppRuleEntry { id, rule });
            id
        }
    }

    /// Remove an app rule.
    pub fn remove_app_rule(&mut self, app: &AppKey) -> Option<AppRuleEntry> {
        self.apps.remove(app)
    }

    /// Add a site rule, merging an equal pattern with the stricter action.
    pub fn set_site_rule(&mut self, pattern: HostPattern, action: SiteAction) -> RuleId {
        if let Some(existing) = self.sites.iter_mut().find(|r| r.pattern == pattern) {
            existing.action = merge_site_action(existing.action, action);
            existing.id
        } else {
            let id = Ulid::new();
            self.sites.push(SiteRule {
                id,
                pattern,
                action,
            });
            id
        }
    }

    /// Add a keyword rule. Duplicates merge their scope.
    pub fn add_keyword(&mut self, text: Keyword, scope: KeywordScope) -> RuleId {
        if let Some(existing) = self
            .keywords
            .iter_mut()
            .find(|r| r.text.as_str() == text.as_str())
        {
            existing.scope = match (existing.scope, scope) {
                (KeywordScope::Both, _) | (_, KeywordScope::Both) => KeywordScope::Both,
                (KeywordScope::Search, KeywordScope::Search) => KeywordScope::Search,
                (KeywordScope::Url, KeywordScope::Url) => KeywordScope::Url,
                _ => KeywordScope::Both,
            };
            existing.id
        } else {
            let id = Ulid::new();
            self.keywords.push(KeywordRule { id, text, scope });
            id
        }
    }

    /// Allow an app.
    pub fn allow_app(&mut self, app: AppKey) {
        self.allow_apps.insert(app);
    }

    /// Allow a host pattern.
    pub fn allow_site(&mut self, pattern: HostPattern) {
        if !self.allow_sites.contains(&pattern) {
            self.allow_sites.push(pattern);
        }
    }

    /// Merge another rule set into this one without removing existing items.
    ///
    /// Duplicate rules keep the stricter outcome; allowlist entries are unioned.
    pub fn merge_from(&mut self, other: &RuleSet) {
        for (app, entry) in &other.apps {
            self.set_app_rule(app.clone(), entry.rule.clone());
        }
        for site in &other.sites {
            self.set_site_rule(site.pattern.clone(), site.action);
        }
        for keyword in &other.keywords {
            self.add_keyword(keyword.text.clone(), keyword.scope);
        }
        for app in &other.allow_apps {
            self.allow_app(app.clone());
        }
        for site in &other.allow_sites {
            self.allow_site(site.clone());
        }
        if let Some(incoming) = other.pickup_limit {
            self.pickup_limit = Some(match self.pickup_limit {
                Some(existing) if existing.count <= incoming.count => existing,
                _ => incoming,
            });
        }
    }

    /// Whether the app is explicitly allowed.
    pub fn allows_app(&self, app: &AppKey) -> bool {
        self.allow_apps.contains(app)
    }

    /// Whether the host is explicitly allowed.
    pub fn allows_site(&self, host: &Host) -> bool {
        self.allow_sites.iter().any(|pattern| pattern.matches(host))
    }
}

fn merge_site_action(current: SiteAction, incoming: SiteAction) -> SiteAction {
    use SiteAction::*;
    match (current, incoming) {
        (Block, _) | (_, Block) => Block,
        (Adult, Adult) => Adult,
        (Adult, Limit(_)) | (Limit(_), Adult) => Adult,
        (Limit(a), Limit(b)) => Limit(a.min(b)),
    }
}

/// A named bundle of rules and an optional schedule.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Profile {
    /// Stable identity.
    pub id: ProfileId,
    /// Unique, case-insensitively compared name.
    pub name: String,
    /// Optional colour.
    pub color: Option<Rgb>,
    /// Optional icon.
    pub icon: Option<IconRef>,
    /// Whether the profile participates in decisions at all.
    pub enabled: bool,
    /// 0..=9; higher wins when explaining overlapping blocks.
    pub priority: u8,
    /// Rules.
    pub rules: RuleSet,
    /// Schedule for `Scheduled` profiles.
    pub schedule: Option<Schedule>,
    /// Activation kind.
    pub activation: Activation,
    /// Pause state.
    pub pause: PauseState,
    /// Included in strict-mode profile selection.
    pub strict_eligible: bool,
    /// Template the profile was created from.
    pub template: Option<TemplateId>,
    /// Creation time.
    pub created_at: Timestamp,
    /// Last edit time.
    pub updated_at: Timestamp,
}

impl Profile {
    /// Create a manual profile with no rules.
    pub fn new(name: impl Into<String>, created_at: Timestamp) -> Result<Self> {
        let name = name.into().trim().to_owned();
        if name.is_empty() {
            return Err(Error::EmptyProfileName);
        }
        if name.chars().count() > 64 {
            return Err(Error::ProfileNameTooLong);
        }
        Ok(Self {
            id: Ulid::new(),
            name,
            color: None,
            icon: None,
            enabled: true,
            priority: 5,
            rules: RuleSet::default(),
            schedule: None,
            activation: Activation::Manual,
            pause: PauseState::default(),
            strict_eligible: false,
            template: None,
            created_at,
            updated_at: created_at,
        })
    }

    /// Use an explicit identifier (tests and imports).
    pub fn with_id(mut self, id: ProfileId) -> Self {
        self.id = id;
        self
    }

    /// Set the activation kind.
    pub fn with_activation(mut self, activation: Activation) -> Self {
        self.activation = activation;
        self
    }

    /// Attach a schedule.
    pub fn with_schedule(mut self, schedule: Schedule) -> Self {
        self.schedule = Some(schedule);
        self
    }

    /// Mark strict-mode eligibility.
    pub fn with_strict_eligible(mut self, eligible: bool) -> Self {
        self.strict_eligible = eligible;
        self
    }

    /// Set priority.
    pub fn with_priority(mut self, priority: u8) -> Self {
        self.priority = priority.min(9);
        self
    }

    /// Replace the rule set.
    pub fn with_rules(mut self, rules: RuleSet) -> Self {
        self.rules = rules;
        self
    }

    /// Set the start-of-day override used by this profile's schedules.
    pub fn start_of_day_override(&self) -> Option<NaiveTime> {
        self.schedule
            .as_ref()
            .and_then(|schedule| schedule.start_of_day)
    }

    /// Block an app.
    pub fn block_app(mut self, app: impl AsRef<str>) -> Result<Self> {
        let app = AppKey::new(app)?;
        self.rules.set_app_rule(app, AppRule::Block);
        Ok(self)
    }

    /// Allow an app.
    pub fn allow_app(mut self, app: impl AsRef<str>) -> Result<Self> {
        self.rules.allow_app(AppKey::new(app)?);
        Ok(self)
    }

    /// Apply a daily limit to an app.
    pub fn daily_limit(mut self, app: impl AsRef<str>, limit: Duration) -> Result<Self> {
        if limit.is_zero() {
            return Err(Error::InvalidRule("daily limit must be positive".into()));
        }
        let app = AppKey::new(app)?;
        self.rules.set_app_rule(app, AppRule::DailyLimit { limit });
        Ok(self)
    }

    /// Apply an hourly limit to an app.
    pub fn hourly_limit(mut self, app: impl AsRef<str>, limit: Duration) -> Result<Self> {
        if limit.is_zero() {
            return Err(Error::InvalidRule("hourly limit must be positive".into()));
        }
        let app = AppKey::new(app)?;
        self.rules.set_app_rule(app, AppRule::HourlyLimit { limit });
        Ok(self)
    }

    /// Apply a launch-count limit to an app.
    pub fn launch_limit(
        mut self,
        app: impl AsRef<str>,
        count: u32,
        per: LimitPeriod,
    ) -> Result<Self> {
        let limit = CountLimit::new(count, per)?;
        let app = AppKey::new(app)?;
        self.rules.set_app_rule(
            app,
            AppRule::LaunchLimit {
                count: limit.count,
                per: limit.per,
            },
        );
        Ok(self)
    }

    /// Apply a wait timer to an app.
    pub fn wait_timer(
        mut self,
        app: impl AsRef<str>,
        use_for: Duration,
        wait: Duration,
        adaptive: bool,
    ) -> Result<Self> {
        if use_for.is_zero() || wait.is_zero() {
            return Err(Error::InvalidRule(
                "wait timer durations must be positive".into(),
            ));
        }
        let app = AppKey::new(app)?;
        self.rules.set_app_rule(
            app,
            AppRule::WaitTimer {
                use_for,
                wait,
                adaptive,
            },
        );
        Ok(self)
    }

    /// Set a global pickup limit for this profile.
    pub fn with_pickup_limit(mut self, count: u32, per: LimitPeriod) -> Result<Self> {
        self.rules.pickup_limit = Some(CountLimit::new(count, per)?);
        Ok(self)
    }

    /// Block a site pattern.
    pub fn block_site(mut self, pattern: impl AsRef<str>) -> Result<Self> {
        self.rules
            .set_site_rule(HostPattern::new(pattern)?, SiteAction::Block);
        Ok(self)
    }

    /// Allow a site pattern.
    pub fn allow_site(mut self, pattern: impl AsRef<str>) -> Result<Self> {
        self.rules.allow_site(HostPattern::new(pattern)?);
        Ok(self)
    }

    /// Block a keyword.
    pub fn block_keyword(mut self, text: impl AsRef<str>) -> Result<Self> {
        self.rules
            .add_keyword(Keyword::new(text)?, KeywordScope::default());
        Ok(self)
    }

    /// Validate invariants that do not depend on other profiles.
    pub fn validate(&self) -> Result<()> {
        if self.name.trim().is_empty() {
            return Err(Error::EmptyProfileName);
        }
        if self.name.chars().count() > 64 {
            return Err(Error::ProfileNameTooLong);
        }
        if self.priority > 9 {
            return Err(Error::InvalidRule("profile priority must be 0..=9".into()));
        }
        if self.activation == Activation::Scheduled && self.schedule.is_none() {
            return Err(Error::MissingSchedule);
        }
        if let Some(schedule) = &self.schedule {
            schedule.validate()?;
        }
        Ok(())
    }
}

/// A quick block target.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Target {
    /// A desktop application.
    App(AppKey),
    /// A website host.
    Site(Host),
}

impl Target {
    /// The stable key used in logs.
    pub fn key(&self) -> &str {
        match self {
            Self::App(app) => app.as_str(),
            Self::Site(host) => host.as_str(),
        }
    }
}

impl std::fmt::Display for Target {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.key())
    }
}

/// An ad-hoc block with a mandatory end.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuickBlock {
    /// What is blocked.
    pub target: Target,
    /// When the block ends.
    pub until: Timestamp,
    /// When it was created.
    pub created_at: Timestamp,
}

/// Strict or lock commitment.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CommitmentKind {
    /// Time-boxed commitment with a deactivation method.
    Strict,
    /// Lock mode; exit normally requires the password.
    Lock,
}

/// Commitment state machine.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CommitmentState {
    /// Configured but not yet enforcing.
    Armed,
    /// Enforcing.
    Active,
    /// Deactivation requested; waiting out the cool-down.
    CoolingDown,
    /// Method satisfied; may end.
    ReadyToEnd,
    /// No longer enforcing.
    Ended,
}

/// How a commitment may be ended.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DeactivationMethod {
    /// Wait out a cool-down.
    Cooldown(Duration),
    /// Type an exact phrase.
    Text(String),
    /// Scan a saved QR code.
    Qr(String),
    /// A trusted friend approves.
    Approval(String),
}

/// Restrictions that a commitment can add.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Restriction {
    /// Cover the desktop settings window.
    BlockSettings,
    /// Reject profile, rule and allowlist edits.
    BlockEdits,
    /// Require a password to open the UI.
    PasswordGate,
    /// Disable pause and break actions.
    BlockPause,
}

/// A durable commitment.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Commitment {
    /// Stable identity.
    pub id: CommitmentId,
    /// Strict or lock.
    pub kind: CommitmentKind,
    /// Current state.
    pub state: CommitmentState,
    /// When it was created.
    pub starts_at: Timestamp,
    /// Wall-clock expiry; `None` means until the method is satisfied.
    pub ends_at: Option<Timestamp>,
    /// Deactivation method.
    pub method: DeactivationMethod,
    /// Extra restrictions.
    pub restrictions: BTreeSet<Restriction>,
    /// Apps allowed even while committed.
    pub allow_apps: BTreeSet<AppKey>,
    /// Sites allowed even while committed.
    pub allow_sites: Vec<HostPattern>,
    /// When it entered its current state.
    pub updated_at: Timestamp,
    /// When a requested cool-down finishes; `None` unless cooling down.
    #[serde(default)]
    pub cooldown_ends_at: Option<Timestamp>,
}

impl Commitment {
    /// Create an active commitment.
    pub fn new(
        kind: CommitmentKind,
        starts_at: Timestamp,
        ends_at: Option<Timestamp>,
        method: DeactivationMethod,
    ) -> Self {
        Self {
            id: Ulid::new(),
            kind,
            state: CommitmentState::Active,
            starts_at,
            ends_at,
            method,
            restrictions: BTreeSet::new(),
            allow_apps: BTreeSet::new(),
            allow_sites: Vec::new(),
            updated_at: starts_at,
            cooldown_ends_at: None,
        }
    }

    /// Add restrictions. Only allowed before the commitment becomes active
    /// through the persisted state machine; this constructor starts active.
    pub fn with_restrictions(
        mut self,
        restrictions: impl IntoIterator<Item = Restriction>,
    ) -> Self {
        self.restrictions.extend(restrictions);
        self
    }

    /// Allow an app during the commitment.
    pub fn allowing_app(mut self, app: impl AsRef<str>) -> Result<Self> {
        self.allow_apps.insert(AppKey::new(app)?);
        Ok(self)
    }

    /// Whether the commitment is enforcing at `now`.
    ///
    /// Expiry is wall-clock: time spent powered off counts. An expired
    /// commitment is inert even if the daemon has not yet written `Ended`.
    pub fn is_enforcing(&self, now: Timestamp) -> bool {
        match self.state {
            CommitmentState::Active
            | CommitmentState::CoolingDown
            | CommitmentState::ReadyToEnd => self.ends_at.is_none_or(|end| now < end),
            CommitmentState::Armed | CommitmentState::Ended => false,
        }
    }

    /// Request deactivation. Returns the instant the cool-down ends.
    ///
    /// Text/QR/approval challenges are not implemented yet; those methods
    /// return [`Error::UnsupportedCommitmentMethod`] rather than silently
    /// ending the commitment.
    pub fn request_end(&mut self, now: Timestamp) -> Result<Timestamp> {
        // Repeating the request must not extend an existing cool-down.
        if self.state == CommitmentState::CoolingDown {
            return self.cooldown_ends_at.ok_or(Error::CommitmentNotReady);
        }
        if self.state != CommitmentState::Active || !self.is_enforcing(now) {
            return Err(Error::CommitmentNotReady);
        }
        match self.method {
            DeactivationMethod::Cooldown(duration) => {
                let delta = chrono::TimeDelta::from_std(duration)
                    .map_err(|_| Error::InvalidDuration("cooldown is too large".into()))?;
                let end = now + delta;
                self.state = CommitmentState::CoolingDown;
                self.cooldown_ends_at = Some(end);
                self.updated_at = now;
                Ok(end)
            }
            _ => Err(Error::UnsupportedCommitmentMethod),
        }
    }

    /// Finish deactivation once the cool-down (or the commitment itself) has
    /// expired.
    pub fn finish_end(&mut self, now: Timestamp) -> Result<()> {
        let commitment_expired = self.ends_at.is_some_and(|end| now >= end);
        let cooldown_finished = self.cooldown_ends_at.is_some_and(|end| now >= end);
        if self.state == CommitmentState::CoolingDown && (commitment_expired || cooldown_finished) {
            self.state = CommitmentState::Ended;
            self.updated_at = now;
            Ok(())
        } else {
            Err(Error::CommitmentNotReady)
        }
    }

    /// Whether editing rules is forbidden.
    pub fn blocks_edits(&self, now: Timestamp) -> bool {
        self.is_enforcing(now) && self.restrictions.contains(&Restriction::BlockEdits)
    }

    /// Whether pause/break is forbidden.
    pub fn blocks_pause(&self, now: Timestamp) -> bool {
        self.is_enforcing(now) && self.restrictions.contains(&Restriction::BlockPause)
    }

    /// Whether the target is explicitly allowed by this commitment.
    pub fn allows(&self, target: &Target) -> bool {
        match target {
            Target::App(app) => self.allow_apps.contains(app),
            Target::Site(host) => self.allow_sites.iter().any(|pattern| pattern.matches(host)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use chrono::Utc;

    fn now() -> Timestamp {
        Utc.with_ymd_and_hms(2026, 10, 22, 9, 0, 0).unwrap()
    }

    #[test]
    fn duplicate_app_rules_merge_to_the_stricter_outcome() {
        let mut rules = RuleSet::default();
        let app = AppKey::new("firefox").unwrap();
        let id = rules.set_app_rule(
            app.clone(),
            AppRule::DailyLimit {
                limit: Duration::from_secs(3600),
            },
        );
        let id2 = rules.set_app_rule(
            app.clone(),
            AppRule::DailyLimit {
                limit: Duration::from_secs(1800),
            },
        );
        assert_eq!(id, id2);
        assert_eq!(
            rules.apps[&app].rule,
            AppRule::DailyLimit {
                limit: Duration::from_secs(1800)
            }
        );
        rules.set_app_rule(app.clone(), AppRule::Block);
        assert_eq!(rules.apps[&app].rule, AppRule::Block);
    }

    #[test]
    fn profile_names_are_validated() {
        assert!(Profile::new("", now()).is_err());
        assert!(Profile::new("  ", now()).is_err());
        assert!(Profile::new("x".repeat(65), now()).is_err());
        assert!(Profile::new("Focus", now()).is_ok());
    }

    #[test]
    fn scheduled_profile_requires_a_schedule() {
        let profile = Profile::new("Focus", now())
            .unwrap()
            .with_activation(Activation::Scheduled);
        assert_eq!(profile.validate(), Err(Error::MissingSchedule));
    }

    #[test]
    fn pause_expires_at_its_deadline() {
        let start = now();
        let mut pause =
            PauseState::until(start + chrono::TimeDelta::minutes(15), PauseReason::User);
        assert!(pause.is_paused(start));
        assert!(pause.is_paused(start + chrono::TimeDelta::minutes(14)));
        assert!(!pause.is_paused(start + chrono::TimeDelta::minutes(15)));
        assert!(pause.expire(start + chrono::TimeDelta::minutes(15)));
        assert!(!pause.paused);
    }

    #[test]
    fn commitment_is_wall_clock_and_cannot_be_shortened_by_restart() {
        let start = now();
        let commitment = Commitment::new(
            CommitmentKind::Strict,
            start,
            Some(start + chrono::TimeDelta::hours(1)),
            DeactivationMethod::Cooldown(Duration::from_secs(600)),
        );
        assert!(commitment.is_enforcing(start));
        assert!(commitment.is_enforcing(start + chrono::TimeDelta::minutes(59)));
        assert!(!commitment.is_enforcing(start + chrono::TimeDelta::hours(1)));
    }
}
