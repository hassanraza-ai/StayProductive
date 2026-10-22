//! The decision engine.
//!
//! [`decide`] is a pure function of `(state, event, now)`. It is the only place
//! that decides whether a target is blocked. Observers provide facts through
//! [`State::observe`]; adapters enforce the returned [`Decision`].
//!
//! Precedence follows `docs/spec/blocking-engine.md` §3 exactly. In particular,
//! commitments and strict-eligible profile windows are checked before every
//! allowlist, pause, and limit.

use std::cmp::Reverse;
use std::collections::BTreeMap;
use std::fmt;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::app::{AppKey, Host, SurfaceId, WindowKey};
use crate::domain::{
    Activation, AppRule, Commitment, CountLimit, LimitPeriod, PauseReason, PauseState, Profile,
    QuickBlock, SiteAction, Target,
};
use crate::error::{Error, Result};
use crate::ids::{CommitmentId, ProfileId, RuleId};
use crate::period::{ClockSettings, Period, day_period, hour_period};
use crate::reason::ReasonCode;
use crate::schedule::ScheduleStatus;
use crate::time::Timestamp;
use crate::usage::UsageLedger;

/// The verdict for a target.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Verdict {
    /// Allowed with no rule involved.
    Allow,
    /// Blocked indefinitely or until a condition that is not a timestamp.
    Block,
    /// Blocked until `Decision::until`.
    BlockUntil,
    /// Allowed, but the UI should show a notice (goal credit).
    AllowWithNotice,
}

impl Verdict {
    /// Whether enforcement should hide/cover the target.
    pub const fn is_block(self) -> bool {
        matches!(self, Self::Block | Self::BlockUntil)
    }
}

/// A translation key for user-facing text.
///
/// The UI resolves this through its catalogs; the engine never builds sentences.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MessageKey(&'static str);

impl MessageKey {
    /// Resolve the key for a reason code.
    pub const fn for_reason(reason: ReasonCode) -> Self {
        Self(match reason {
            ReasonCode::Strict => "block.strict",
            ReasonCode::Profile => "block.profile",
            ReasonCode::LimitDaily => "block.limit.daily",
            ReasonCode::LimitHourly => "block.limit.hourly",
            ReasonCode::Launch => "block.launch",
            ReasonCode::Pickup => "block.pickup",
            ReasonCode::Wait => "block.wait",
            ReasonCode::Keyword => "block.keyword",
            ReasonCode::Site => "block.site",
            ReasonCode::Surface => "block.surface",
            ReasonCode::Quick => "block.quick",
            ReasonCode::Allow => "allow.override",
            ReasonCode::Pause => "allow.paused",
            ReasonCode::NoRule => "allow.none",
        })
    }

    /// The stable key.
    pub const fn as_str(self) -> &'static str {
        self.0
    }
}

impl fmt::Display for MessageKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}

/// A decision about one target.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Decision {
    /// What to do.
    pub verdict: Verdict,
    /// Why.
    pub reason: ReasonCode,
    /// The rule that fired, when it has an identity.
    pub rule: Option<RuleId>,
    /// The profile that supplied the deciding rule.
    pub profile: Option<ProfileId>,
    /// The target the decision is about.
    pub target: Option<Target>,
    /// Known end of the block or notice, if any.
    pub until: Option<Timestamp>,
    /// Translation key for the explanation.
    pub message: MessageKey,
}

impl Decision {
    /// Whether this decision blocks enforcement.
    pub fn is_block(&self) -> bool {
        self.verdict.is_block()
    }
}

/// A normalized observation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Event {
    /// The compositor reported a new foreground app.
    ForegroundChanged {
        /// Foreground application.
        app: AppKey,
        /// Window, when the adapter knows it.
        window: Option<WindowKey>,
    },
    /// The extension reported a navigation.
    UrlChanged {
        /// Browser application.
        app: AppKey,
        /// Full URL.
        url: String,
        /// Page title, when available.
        title: Option<String>,
    },
    /// An in-page content surface became visible or hidden.
    SurfaceChanged {
        /// Browser application.
        app: AppKey,
        /// Surface identifier.
        surface: SurfaceId,
        /// Whether it is visible.
        visible: bool,
    },
    /// The timer wheel fired.
    Tick,
    /// The session locked.
    ScreenLocked,
    /// The session unlocked.
    ScreenUnlocked,
    /// The session suspended.
    SessionSuspended,
    /// The session resumed.
    Resumed,
    /// Settings changed; compiled schedules are invalidated.
    SettingsChanged,
    /// A commitment changed state.
    CommitmentChanged {
        /// Commitment.
        id: CommitmentId,
    },
    /// The daemon started; re-evaluate everything.
    DaemonStarted,
    /// Wall time jumped by more than the tolerance.
    ClockJumped {
        /// Previous wall time.
        old: Timestamp,
        /// New wall time.
        new: Timestamp,
    },
}

impl Event {
    /// The target this event is about, if any.
    pub fn target(&self, state: &State) -> Option<Target> {
        match self {
            Self::ForegroundChanged { app, .. } | Self::SurfaceChanged { app, .. } => {
                Some(Target::App(app.clone()))
            }
            Self::UrlChanged { url, .. } => Host::from_url(url).map(Target::Site),
            Self::Tick => state.usage.current_app().cloned().map(Target::App),
            _ => None,
        }
    }

    fn url_text(&self) -> Option<&str> {
        match self {
            Self::UrlChanged { url, .. } => Some(url),
            _ => None,
        }
    }

    fn keyword_text(&self, scope: crate::app::KeywordScope) -> Option<String> {
        let url = self.url_text()?;
        Some(match scope {
            crate::app::KeywordScope::Url | crate::app::KeywordScope::Both => url.to_owned(),
            crate::app::KeywordScope::Search => url
                .split_once('?')
                .map(|(_, query)| query.to_owned())
                .unwrap_or_default(),
        })
    }
}

/// The engine's runtime state.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    /// Clock settings.
    pub settings: ClockSettings,
    /// Profiles, in stable creation order.
    pub profiles: Vec<Profile>,
    /// The single manually activated profile, if any.
    pub manual_active: Option<ProfileId>,
    /// Commitments, including ended history.
    pub commitments: Vec<Commitment>,
    /// Active ad-hoc blocks.
    pub quick_blocks: Vec<QuickBlock>,
    /// Extensions to scheduled blocks.
    pub extensions: BTreeMap<ProfileId, Timestamp>,
    /// Usage ledger.
    pub usage: UsageLedger,
    /// Whether the session is locked.
    pub screen_locked: bool,
}

impl State {
    /// An empty state with the supplied clock settings.
    pub fn new(settings: ClockSettings) -> Self {
        Self {
            settings,
            profiles: Vec::new(),
            manual_active: None,
            commitments: Vec::new(),
            quick_blocks: Vec::new(),
            extensions: BTreeMap::new(),
            usage: UsageLedger::new(),
            screen_locked: false,
        }
    }

    /// A deterministic fixture state for tests.
    pub fn fixture() -> Self {
        Self::new(ClockSettings::default())
    }

    /// Add a profile, enforcing name uniqueness.
    pub fn add_profile(&mut self, profile: Profile) -> Result<ProfileId> {
        profile.validate()?;
        let name = profile.name.to_lowercase();
        if self
            .profiles
            .iter()
            .any(|existing| existing.name.to_lowercase() == name)
        {
            return Err(Error::DuplicateProfileName(profile.name));
        }
        let id = profile.id;
        self.profiles.push(profile);
        Ok(id)
    }

    /// Find a profile by identity.
    pub fn profile(&self, id: ProfileId) -> Option<&Profile> {
        self.profiles.iter().find(|profile| profile.id == id)
    }

    /// Find a profile by name, case-insensitively.
    pub fn profile_by_name(&self, name: &str) -> Option<&Profile> {
        self.profiles
            .iter()
            .find(|profile| profile.name.eq_ignore_ascii_case(name))
    }

    /// Mutable profile lookup.
    pub fn profile_mut(&mut self, id: ProfileId) -> Option<&mut Profile> {
        self.profiles.iter_mut().find(|profile| profile.id == id)
    }

    /// Rename a profile, preserving uniqueness.
    pub fn rename_profile(
        &mut self,
        id: ProfileId,
        name: impl Into<String>,
        now: Timestamp,
    ) -> Result<()> {
        let name = name.into().trim().to_owned();
        if name.is_empty() {
            return Err(Error::EmptyProfileName);
        }
        if name.chars().count() > 64 {
            return Err(Error::ProfileNameTooLong);
        }
        if self
            .profiles
            .iter()
            .any(|other| other.id != id && other.name.eq_ignore_ascii_case(&name))
        {
            return Err(Error::DuplicateProfileName(name));
        }
        let profile = self.profile_mut(id).ok_or(Error::UnknownProfile(id))?;
        profile.name = name;
        profile.updated_at = now;
        Ok(())
    }

    /// Duplicate a profile. History is not copied.
    pub fn duplicate_profile(
        &mut self,
        id: ProfileId,
        new_name: impl Into<String>,
        now: Timestamp,
    ) -> Result<ProfileId> {
        let source = self.profile(id).ok_or(Error::UnknownProfile(id))?;
        let mut copy = source.clone();
        copy.id = crate::ids::new_id();
        copy.name = new_name.into();
        copy.created_at = now;
        copy.updated_at = now;
        copy.pause = PauseState::default();
        copy.rules = {
            // Rule identities are per-profile; regenerate them so logs and
            // edits never point at the source profile's rules.
            let mut rules = crate::domain::RuleSet::default();
            for (app, entry) in &source.rules.apps {
                rules.set_app_rule(app.clone(), entry.rule.clone());
            }
            for site in &source.rules.sites {
                rules.set_site_rule(site.pattern.clone(), site.action);
            }
            for keyword in &source.rules.keywords {
                rules.add_keyword(keyword.text.clone(), keyword.scope);
            }
            for app in &source.rules.allow_apps {
                rules.allow_app(app.clone());
            }
            for site in &source.rules.allow_sites {
                rules.allow_site(site.clone());
            }
            rules.pickup_limit = source.rules.pickup_limit;
            rules
        };
        self.add_profile(copy)
    }

    /// Remove a profile. Usage events and commitment history are retained.
    pub fn remove_profile(&mut self, id: ProfileId) -> Result<()> {
        let before = self.profiles.len();
        self.profiles.retain(|profile| profile.id != id);
        if self.profiles.len() == before {
            return Err(Error::UnknownProfile(id));
        }
        if self.manual_active == Some(id) {
            self.manual_active = None;
        }
        self.extensions.remove(&id);
        Ok(())
    }

    /// Copy rules from one profile into another, keeping the target's items.
    pub fn copy_items(
        &mut self,
        source: ProfileId,
        target: ProfileId,
        now: Timestamp,
    ) -> Result<()> {
        if self
            .commitments
            .iter()
            .any(|commitment| commitment.blocks_edits(now))
        {
            return Err(Error::CommitmentBlocksAction);
        }
        let source_rules = self
            .profile(source)
            .ok_or(Error::UnknownProfile(source))?
            .rules
            .clone();
        let target_profile = self
            .profile_mut(target)
            .ok_or(Error::UnknownProfile(target))?;
        target_profile.rules.merge_from(&source_rules);
        target_profile.updated_at = now;
        Ok(())
    }

    /// Close a foreground interval left open by a crash, at the last heartbeat.
    pub fn recover(&mut self) {
        self.usage.recover();
    }

    /// Manually activate a profile. Exactly one manual profile can be active.
    pub fn activate_manual(&mut self, id: ProfileId, now: Timestamp) -> Result<()> {
        let profile = self.profile_mut(id).ok_or(Error::UnknownProfile(id))?;
        profile.enabled = true;
        profile.pause.resume();
        profile.updated_at = now;
        self.manual_active = Some(id);
        Ok(())
    }

    /// Deactivate the manual profile.
    pub fn deactivate_manual(&mut self, now: Timestamp) {
        if let Some(id) = self.manual_active.take() {
            if let Some(profile) = self.profile_mut(id) {
                profile.updated_at = now;
            }
        }
    }

    /// Pause a profile until `until` (`None` = indefinite).
    pub fn pause_profile(
        &mut self,
        id: ProfileId,
        until: Option<Timestamp>,
        reason: PauseReason,
        now: Timestamp,
    ) -> Result<()> {
        if self
            .commitments
            .iter()
            .any(|commitment| commitment.blocks_pause(now))
        {
            return Err(Error::CommitmentBlocksAction);
        }
        if until.is_some_and(|until| until <= now) {
            return Err(Error::InvalidDuration(
                "pause must end in the future".into(),
            ));
        }
        let profile = self.profile_mut(id).ok_or(Error::UnknownProfile(id))?;
        profile.pause = match until {
            Some(until) => PauseState::until(until, reason),
            None => PauseState::indefinite(reason),
        };
        profile.updated_at = now;
        self.usage.paused(now);
        Ok(())
    }

    /// Resume a paused profile immediately.
    pub fn resume_profile(&mut self, id: ProfileId, now: Timestamp) -> Result<()> {
        let profile = self.profile_mut(id).ok_or(Error::UnknownProfile(id))?;
        profile.pause.resume();
        profile.updated_at = now;
        self.usage.resumed_from_pause(now);
        Ok(())
    }

    /// Add a quick block. The end is mandatory and must be in the future.
    pub fn quick_block(&mut self, target: Target, until: Timestamp, now: Timestamp) -> Result<()> {
        if until <= now {
            return Err(Error::InvalidDuration(
                "quick block must end in the future".into(),
            ));
        }
        self.quick_blocks.push(QuickBlock {
            target,
            until,
            created_at: now,
        });
        Ok(())
    }

    /// Remove quick blocks for a target. Used when a user ends one early.
    pub fn clear_quick_block(&mut self, target: &Target) {
        self.quick_blocks.retain(|block| &block.target != target);
    }

    /// Add a commitment. It starts enforcing immediately.
    pub fn add_commitment(&mut self, commitment: Commitment) -> Result<()> {
        if commitment
            .ends_at
            .is_some_and(|end| end <= commitment.starts_at)
        {
            return Err(Error::InvalidDuration(
                "commitment must end after it starts".into(),
            ));
        }
        self.commitments.push(commitment);
        Ok(())
    }

    /// Whether an active commitment forbids changing rules.
    pub fn edits_blocked(&self, now: Timestamp) -> bool {
        self.commitments
            .iter()
            .any(|commitment| commitment.blocks_edits(now))
    }

    /// The active commitment with the latest end, if any.
    pub fn active_commitment(&self, now: Timestamp) -> Option<&Commitment> {
        self.commitments
            .iter()
            .filter(|commitment| commitment.is_enforcing(now))
            .max_by_key(|commitment| {
                (
                    commitment.kind == crate::domain::CommitmentKind::Lock,
                    commitment.ends_at,
                )
            })
    }

    /// Request deactivation of a commitment, starting its cool-down.
    pub fn request_commitment_end(
        &mut self,
        id: CommitmentId,
        now: Timestamp,
    ) -> Result<Timestamp> {
        let commitment = self
            .commitments
            .iter_mut()
            .find(|commitment| commitment.id == id)
            .ok_or(Error::UnknownCommitment(id))?;
        commitment.request_end(now)
    }

    /// Finish deactivation after the cool-down has elapsed.
    pub fn finish_commitment_end(&mut self, id: CommitmentId, now: Timestamp) -> Result<()> {
        let commitment = self
            .commitments
            .iter_mut()
            .find(|commitment| commitment.id == id)
            .ok_or(Error::UnknownCommitment(id))?;
        commitment.finish_end(now)
    }

    /// Extend the active block of `id` by `duration`.
    ///
    /// Extending is always forward; it can never shorten a block.
    pub fn extend_block(
        &mut self,
        id: ProfileId,
        duration: Duration,
        now: Timestamp,
    ) -> Result<Timestamp> {
        if duration.is_zero() {
            return Err(Error::InvalidDuration("extend must be positive".into()));
        }
        let profile = self.profile(id).ok_or(Error::UnknownProfile(id))?;
        if !self.profile_is_active(profile, now) {
            return Err(Error::InvalidRule(
                "profile is not currently blocking".into(),
            ));
        }
        let delta = chrono::TimeDelta::from_std(duration)
            .map_err(|_| Error::InvalidDuration("extend is too large".into()))?;
        let current_end = self.profile_end(profile, now).unwrap_or(now);
        let new_end = (current_end.max(now) + delta).max(now + delta);
        let entry = self.extensions.entry(id).or_insert(new_end);
        if new_end > *entry {
            *entry = new_end;
        }
        Ok(*entry)
    }

    /// Whether a profile contributes rules at `now`.
    pub fn profile_is_active(&self, profile: &Profile, now: Timestamp) -> bool {
        if !profile.enabled {
            return false;
        }
        let scheduled = match profile.activation {
            Activation::Manual => self.manual_active == Some(profile.id),
            Activation::Always => true,
            Activation::Scheduled => profile
                .schedule
                .as_ref()
                .is_some_and(|schedule| schedule.status(now, self.settings.timezone).active),
        };
        scheduled
            || self
                .extensions
                .get(&profile.id)
                .is_some_and(|end| now < *end)
    }

    /// The known end of a profile's current block, if any.
    pub fn profile_end(&self, profile: &Profile, now: Timestamp) -> Option<Timestamp> {
        let scheduled_end = profile.schedule.as_ref().and_then(|schedule| {
            let status: ScheduleStatus = schedule.status(now, self.settings.timezone);
            status.active.then_some(status.ends_at).flatten()
        });
        let extended_end = self
            .extensions
            .get(&profile.id)
            .copied()
            .filter(|end| now < *end);
        match (scheduled_end, extended_end) {
            (Some(a), Some(b)) => Some(a.max(b)),
            (Some(end), None) | (None, Some(end)) => Some(end),
            (None, None) => None,
        }
    }

    /// Observe an event, updating usage and session state.
    ///
    /// Pause expiry and quick-block expiry are handled lazily by [`decide`],
    /// so a missed tick can never extend a block.
    pub fn observe(&mut self, event: &Event, now: Timestamp) {
        match event {
            Event::ForegroundChanged { app, .. } => {
                if !self.screen_locked {
                    self.usage.foreground(app.clone(), now);
                }
            }
            Event::UrlChanged { app, url, .. } => {
                if let Some(host) = Host::from_url(url) {
                    self.usage.site_visit(Some(app.clone()), host, now);
                }
            }
            Event::Tick | Event::DaemonStarted | Event::SettingsChanged => {
                self.usage.heartbeat(now);
                for profile in &mut self.profiles {
                    profile.pause.expire(now);
                }
                self.quick_blocks.retain(|block| block.until > now);
            }
            Event::ScreenLocked => {
                self.screen_locked = true;
                self.usage.screen_locked(now);
            }
            Event::ScreenUnlocked => {
                self.screen_locked = false;
                self.usage.screen_unlocked(now);
            }
            Event::SessionSuspended => {
                self.usage.suspended(now);
            }
            Event::Resumed => {
                self.usage.resumed(now);
            }
            Event::ClockJumped { old, new } => {
                self.handle_clock_jump(*old, *new);
            }
            Event::SurfaceChanged { .. } | Event::CommitmentChanged { .. } => {}
        }
    }

    /// Log an enforced block for history.
    pub fn record_decision(&mut self, decision: &Decision, now: Timestamp) {
        if decision.is_block() {
            if let Some(target) = &decision.target {
                self.usage.blocked(
                    target.clone(),
                    decision.reason,
                    now,
                    decision.until,
                    decision.profile,
                );
            }
        }
    }

    /// Apply a wall-clock jump.
    ///
    /// A backwards jump closes the open interval at the last heartbeat so time
    /// is never double counted; a forward jump closes at the new time.
    pub fn handle_clock_jump(&mut self, old: Timestamp, new: Timestamp) {
        let close_at = if new < old {
            let heartbeat = self.usage.last_heartbeat().unwrap_or(old);
            heartbeat.min(old).max(old - chrono::TimeDelta::seconds(1))
        } else {
            new
        };
        self.usage.close_foreground(close_at);
        self.usage.heartbeat(new);
    }

    fn settings_for(&self, profile: &Profile) -> ClockSettings {
        let mut settings = self.settings.clone();
        if let Some(start_of_day) = profile.start_of_day_override() {
            settings.start_of_day = start_of_day;
        }
        settings
    }

    fn day_period_for(&self, profile: &Profile, now: Timestamp) -> Period {
        day_period(now, &self.settings_for(profile))
    }

    fn hour_period_now(&self, now: Timestamp) -> Period {
        hour_period(now, self.settings.timezone)
    }
}

/// Decide what to do about the target of `event` at `now`.
pub fn decide(state: &State, event: &Event, now: Timestamp) -> Decision {
    let Some(target) = event.target(state) else {
        return allow(ReasonCode::NoRule, None);
    };

    // No overlay is produced while the session is locked; enforcement resumes
    // on unlock (blocking engine §8.5).
    if state.screen_locked {
        return allow(ReasonCode::NoRule, Some(target));
    }

    let active: Vec<(&Profile, bool)> = state
        .profiles
        .iter()
        .filter(|profile| state.profile_is_active(profile, now))
        .map(|profile| (profile, profile.pause.is_paused(now)))
        .collect();

    let enforcing: Vec<&Commitment> = state
        .commitments
        .iter()
        .filter(|commitment| commitment.is_enforcing(now))
        .collect();

    // 1. Active commitment forbids the target. The commitment locks the rules
    //    that are active right now; it does not invent new ones.
    if !enforcing.is_empty() {
        let rule_hit = active
            .iter()
            .find(|(profile, _)| profile_has_rule_for_target(profile, &target, event));
        if let Some((profile, _)) = rule_hit {
            if !enforcing
                .iter()
                .any(|commitment| commitment.allows(&target))
            {
                let commitment_end = enforcing
                    .iter()
                    .filter_map(|commitment| commitment.ends_at)
                    .max();
                return block(
                    ReasonCode::Strict,
                    commitment_end.or_else(|| state.profile_end(profile, now)),
                    Some(profile.id),
                    None,
                    target,
                );
            }
        }
    }

    // 2. A strict-eligible profile inside its window blocks with R-STRICT even
    //    when another profile is paused.
    for (profile, _) in &active {
        if profile.strict_eligible && profile_has_rule_for_target(profile, &target, event) {
            if let Some(matched) = profile_match(profile, &target, event, state, now) {
                return block(
                    ReasonCode::Strict,
                    matched.until.or_else(|| state.profile_end(profile, now)),
                    Some(profile.id),
                    matched.rule,
                    target,
                );
            }
            // A strict-eligible profile with a matching rule but no exhausted
            // limit still produces R-STRICT, because the profile itself is the
            // commitment.
            return block(
                ReasonCode::Strict,
                state.profile_end(profile, now),
                Some(profile.id),
                None,
                target,
            );
        }
    }

    // 3. A commitment allowlist permits the target.
    if enforcing
        .iter()
        .any(|commitment| commitment.allows(&target))
    {
        return allow(ReasonCode::Allow, Some(target));
    }

    let mut unpaused_matches = Vec::new();
    let mut paused_matches = Vec::new();
    for (profile, paused) in &active {
        if let Some(matched) = profile_match(profile, &target, event, state, now) {
            if *paused {
                paused_matches.push(matched);
            } else {
                unpaused_matches.push(matched);
            }
        }
    }

    // 4. Pause suppresses the paused profile's rules, but only that profile.
    if unpaused_matches.is_empty() && !paused_matches.is_empty() {
        return allow(ReasonCode::Pause, Some(target));
    }

    // 5. Profile allowlists override non-strict rules.
    let allowed = active.iter().any(|(profile, paused)| {
        !*paused
            && match &target {
                Target::App(app) => profile.rules.allows_app(app),
                Target::Site(host) => profile.rules.allows_site(host),
            }
    });
    if allowed {
        return allow(ReasonCode::Allow, Some(target));
    }

    // 6. Quick blocks.
    if let Some(quick) = state
        .quick_blocks
        .iter()
        .find(|block| block.target == target && block.until > now)
    {
        return block(ReasonCode::Quick, Some(quick.until), None, None, target);
    }

    // 7-13. Limits, timers, profile rules, keywords and sites.
    if let Some(best) = choose_match(&unpaused_matches) {
        let until = merge_until(&unpaused_matches);
        let verdict = if until.is_some() {
            Verdict::BlockUntil
        } else {
            Verdict::Block
        };
        return Decision {
            verdict,
            reason: best.reason,
            rule: best.rule,
            profile: Some(best.profile),
            target: Some(target),
            until,
            message: MessageKey::for_reason(best.reason),
        };
    }

    // 14. Nothing matches.
    allow(ReasonCode::NoRule, Some(target))
}

fn allow(reason: ReasonCode, target: Option<Target>) -> Decision {
    Decision {
        verdict: Verdict::Allow,
        reason,
        rule: None,
        profile: None,
        target,
        until: None,
        message: MessageKey::for_reason(reason),
    }
}

fn block(
    reason: ReasonCode,
    until: Option<Timestamp>,
    profile: Option<ProfileId>,
    rule: Option<RuleId>,
    target: Target,
) -> Decision {
    let verdict = if until.is_some() {
        Verdict::BlockUntil
    } else {
        Verdict::Block
    };
    Decision {
        verdict,
        reason,
        rule,
        profile,
        target: Some(target),
        until,
        message: MessageKey::for_reason(reason),
    }
}

#[derive(Clone, Debug)]
struct RuleMatch {
    profile: ProfileId,
    priority: u8,
    rule: Option<RuleId>,
    reason: ReasonCode,
    until: Option<Timestamp>,
}

fn choose_match(matches: &[RuleMatch]) -> Option<&RuleMatch> {
    matches.iter().min_by_key(|matched| {
        (
            Reverse(matched.priority),
            matched.reason.precedence(),
            matched
                .until
                .map(|until| until.timestamp_millis())
                .unwrap_or(i64::MAX),
        )
    })
}

fn merge_until(matches: &[RuleMatch]) -> Option<Timestamp> {
    if matches.iter().any(|matched| matched.until.is_none()) {
        return None;
    }
    matches.iter().filter_map(|matched| matched.until).min()
}

/// Whether a profile has any rule mentioning this target.
///
/// Used by commitments and strict profiles: those block the rule that exists,
/// not only the rule that happens to be exhausted.
fn profile_has_rule_for_target(profile: &Profile, target: &Target, event: &Event) -> bool {
    match target {
        Target::App(app) => profile.rules.apps.contains_key(app),
        Target::Site(host) => {
            profile
                .rules
                .sites
                .iter()
                .any(|rule| rule.pattern.matches(host))
                || profile.rules.keywords.iter().any(|rule| {
                    event
                        .keyword_text(rule.scope)
                        .is_some_and(|text| rule.text.matches(&text))
                })
        }
    }
}

/// The strongest active rule of one profile for a target.
fn profile_match(
    profile: &Profile,
    target: &Target,
    event: &Event,
    state: &State,
    now: Timestamp,
) -> Option<RuleMatch> {
    let mut matches = Vec::new();

    if let Some(limit) = profile.rules.pickup_limit {
        if let Some(matched) = count_limit_match(profile, limit, state, now, RuleKind::Pickup) {
            matches.push(matched);
        }
    }

    match target {
        Target::App(app) => {
            if let Some(entry) = profile.rules.apps.get(app) {
                if let Some(matched) = app_rule_match(profile, app, entry, state, now) {
                    matches.push(matched);
                }
            }
        }
        Target::Site(host) => {
            for rule in &profile.rules.sites {
                if !rule.pattern.matches(host) {
                    continue;
                }
                match rule.action {
                    SiteAction::Block => matches.push(RuleMatch {
                        profile: profile.id,
                        priority: profile.priority,
                        rule: Some(rule.id),
                        reason: ReasonCode::Site,
                        until: state.profile_end(profile, now),
                    }),
                    SiteAction::Adult => matches.push(RuleMatch {
                        profile: profile.id,
                        priority: profile.priority,
                        rule: Some(rule.id),
                        reason: ReasonCode::Site,
                        until: state.profile_end(profile, now),
                    }),
                    // Site time limits require per-site intervals; the engine
                    // reports them through the app limit path once the
                    // extension records site usage. TODO(spec): site limits.
                    SiteAction::Limit(_) => {}
                }
            }
            for rule in &profile.rules.keywords {
                if let Some(text) = event.keyword_text(rule.scope) {
                    if rule.text.matches(&text) {
                        matches.push(RuleMatch {
                            profile: profile.id,
                            priority: profile.priority,
                            rule: Some(rule.id),
                            reason: ReasonCode::Keyword,
                            until: state.profile_end(profile, now),
                        });
                    }
                }
            }
        }
    }

    choose_match(&matches).cloned()
}

#[derive(Clone, Copy)]
enum RuleKind {
    Pickup,
}

fn count_limit_match(
    profile: &Profile,
    limit: CountLimit,
    state: &State,
    now: Timestamp,
    kind: RuleKind,
) -> Option<RuleMatch> {
    let period = match limit.per {
        LimitPeriod::Daily => state.day_period_for(profile, now),
        LimitPeriod::Hourly => state.hour_period_now(now),
    };
    let (count, reason) = match kind {
        RuleKind::Pickup => (state.usage.pickup_count(&period), ReasonCode::Pickup),
    };
    (count >= limit.count).then_some(RuleMatch {
        profile: profile.id,
        priority: profile.priority,
        rule: None,
        reason,
        until: Some(period.end),
    })
}

/// The strongest rule for an app attribute.
fn app_rule_match(
    profile: &Profile,
    app: &AppKey,
    entry: &crate::domain::AppRuleEntry,
    state: &State,
    now: Timestamp,
) -> Option<RuleMatch> {
    let base = |reason: ReasonCode, until: Option<Timestamp>| RuleMatch {
        profile: profile.id,
        priority: profile.priority,
        rule: Some(entry.id),
        reason,
        until,
    };
    let day = state.day_period_for(profile, now);
    let hour = state.hour_period_now(now);

    match &entry.rule {
        AppRule::Block => Some(base(ReasonCode::Profile, state.profile_end(profile, now))),
        AppRule::DailyLimit { limit } => (state.usage.usage(app, &day, now) >= *limit)
            .then(|| base(ReasonCode::LimitDaily, Some(day.end))),
        AppRule::HourlyLimit { limit } => (state.usage.usage(app, &hour, now) >= *limit)
            .then(|| base(ReasonCode::LimitHourly, Some(hour.end))),
        AppRule::LaunchLimit { count, per } => {
            let period = match per {
                LimitPeriod::Daily => day,
                LimitPeriod::Hourly => hour,
            };
            (state.usage.launch_count(app, &period) > *count)
                .then(|| base(ReasonCode::Launch, Some(period.end)))
        }
        AppRule::WaitTimer {
            use_for,
            wait,
            adaptive,
        } => {
            let usage = state.usage.usage(app, &day, now);
            if usage < *use_for || use_for.is_zero() {
                return None;
            }
            let units = (usage.as_millis() / use_for.as_millis().max(1)) as u64;
            let threshold = *use_for * u32::try_from(units).unwrap_or(u32::MAX);
            let crossing = state.usage.usage_crossed_at(app, &day, threshold, now)?;
            let wait_delta = chrono::TimeDelta::from_std(*wait).ok()?;
            let mut end = crossing + wait_delta;
            if *adaptive && now >= crossing + wait_delta / 2 {
                end = crossing + wait_delta / 2;
            }
            (now < end).then(|| base(ReasonCode::Wait, Some(end)))
        }
        // Goal credit is a positive allowance, not a block. The daemon
        // converts it into `AllowWithNotice` after updating credit.
        AppRule::Goal { .. } => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::HostPattern;
    use crate::domain::CommitmentKind;
    use chrono::{TimeZone, Utc};

    fn t(hour: u32, minute: u32) -> Timestamp {
        Utc.with_ymd_and_hms(2026, 10, 22, hour, minute, 0).unwrap()
    }

    fn state_with(profile: Profile) -> State {
        let mut state = State::fixture();
        let id = state.add_profile(profile).unwrap();
        state.activate_manual(id, t(8, 0)).unwrap();
        state
    }

    #[test]
    fn default_is_allow() {
        let state = State::fixture();
        let event = Event::ForegroundChanged {
            app: AppKey::new("firefox").unwrap(),
            window: None,
        };
        let decision = decide(&state, &event, t(9, 0));
        assert_eq!(decision.verdict, Verdict::Allow);
        assert_eq!(decision.reason, ReasonCode::NoRule);
    }

    #[test]
    fn blocked_app_is_r_profile() {
        let profile = Profile::new("Focus", t(8, 0))
            .unwrap()
            .block_app("firefox")
            .unwrap();
        let state = state_with(profile);
        let event = Event::ForegroundChanged {
            app: AppKey::new("firefox").unwrap(),
            window: None,
        };
        let decision = decide(&state, &event, t(9, 0));
        assert_eq!(decision.verdict, Verdict::Block);
        assert_eq!(decision.reason, ReasonCode::Profile);
        assert_eq!(decision.profile, Some(state.manual_active.unwrap()));
    }

    #[test]
    fn allowlist_overrides_profile_rule() {
        let profile = Profile::new("Focus", t(8, 0))
            .unwrap()
            .block_app("firefox")
            .unwrap()
            .allow_app("firefox")
            .unwrap();
        let state = state_with(profile);
        let event = Event::ForegroundChanged {
            app: AppKey::new("firefox").unwrap(),
            window: None,
        };
        assert_eq!(decide(&state, &event, t(9, 0)).reason, ReasonCode::Allow);
    }

    #[test]
    fn site_wildcard_is_overridden_by_specific_allow() {
        let profile = Profile::new("Focus", t(8, 0))
            .unwrap()
            .block_site("*.example.com")
            .unwrap()
            .allow_site("good.example.com")
            .unwrap();
        let state = state_with(profile);
        let blocked = Event::UrlChanged {
            app: AppKey::new("firefox").unwrap(),
            url: "https://bad.example.com/page".into(),
            title: None,
        };
        let allowed = Event::UrlChanged {
            app: AppKey::new("firefox").unwrap(),
            url: "https://good.example.com/page".into(),
            title: None,
        };
        assert_eq!(decide(&state, &blocked, t(9, 0)).reason, ReasonCode::Site);
        assert_eq!(decide(&state, &allowed, t(9, 0)).reason, ReasonCode::Allow);
    }

    #[test]
    fn keyword_blocks_case_insensitively() {
        let profile = Profile::new("Focus", t(8, 0))
            .unwrap()
            .block_keyword("GaMe")
            .unwrap();
        let state = state_with(profile);
        let event = Event::UrlChanged {
            app: AppKey::new("firefox").unwrap(),
            url: "https://example.com/GAME".into(),
            title: None,
        };
        assert_eq!(decide(&state, &event, t(9, 0)).reason, ReasonCode::Keyword);
    }

    #[test]
    fn strict_eligible_profile_blocks_even_when_another_profile_is_paused() {
        let strict = Profile::new("Strict", t(8, 0))
            .unwrap()
            .with_activation(Activation::Always)
            .with_strict_eligible(true)
            .block_app("firefox")
            .unwrap();
        let other = Profile::new("Other", t(8, 0))
            .unwrap()
            .with_activation(Activation::Always)
            .block_app("firefox")
            .unwrap();
        let mut state = State::fixture();
        let other_id = state.add_profile(other).unwrap();
        state.add_profile(strict).unwrap();
        state
            .pause_profile(other_id, None, PauseReason::User, t(9, 0))
            .unwrap();
        let event = Event::ForegroundChanged {
            app: AppKey::new("firefox").unwrap(),
            window: None,
        };
        assert_eq!(decide(&state, &event, t(10, 0)).reason, ReasonCode::Strict);
    }

    #[test]
    fn commitment_ignores_allowlist() {
        let profile = Profile::new("Focus", t(8, 0))
            .unwrap()
            .with_activation(Activation::Always)
            .block_app("firefox")
            .unwrap()
            .allow_app("firefox")
            .unwrap();
        let mut state = State::fixture();
        state.add_profile(profile).unwrap();
        state
            .add_commitment(Commitment::new(
                CommitmentKind::Strict,
                t(8, 0),
                Some(t(18, 0)),
                crate::domain::DeactivationMethod::Cooldown(Duration::from_secs(600)),
            ))
            .unwrap();
        let event = Event::ForegroundChanged {
            app: AppKey::new("firefox").unwrap(),
            window: None,
        };
        let decision = decide(&state, &event, t(10, 0));
        assert_eq!(decision.reason, ReasonCode::Strict);
        assert_eq!(decision.until, Some(t(18, 0)));
    }

    #[test]
    fn paused_profile_allows_with_r_pause() {
        let profile = Profile::new("Focus", t(8, 0))
            .unwrap()
            .block_app("firefox")
            .unwrap();
        let mut state = State::fixture();
        let id = state.add_profile(profile).unwrap();
        state.activate_manual(id, t(8, 0)).unwrap();
        state
            .pause_profile(id, Some(t(9, 15)), PauseReason::User, t(9, 0))
            .unwrap();
        let event = Event::ForegroundChanged {
            app: AppKey::new("firefox").unwrap(),
            window: None,
        };
        assert_eq!(decide(&state, &event, t(9, 5)).reason, ReasonCode::Pause);
        assert_eq!(decide(&state, &event, t(9, 15)).reason, ReasonCode::Profile);
    }

    #[test]
    fn quick_block_ends_without_editing_the_profile() {
        let mut state = State::fixture();
        state
            .quick_block(
                Target::App(AppKey::new("firefox").unwrap()),
                t(9, 10),
                t(9, 0),
            )
            .unwrap();
        let event = Event::ForegroundChanged {
            app: AppKey::new("firefox").unwrap(),
            window: None,
        };
        assert_eq!(decide(&state, &event, t(9, 5)).reason, ReasonCode::Quick);
        assert_eq!(decide(&state, &event, t(9, 10)).reason, ReasonCode::NoRule);
    }

    #[test]
    fn duplicate_profile_names_are_rejected_case_insensitively() {
        let mut state = State::fixture();
        state
            .add_profile(Profile::new("Focus", t(8, 0)).unwrap())
            .unwrap();
        assert_eq!(
            state.add_profile(Profile::new("focus", t(8, 0)).unwrap()),
            Err(Error::DuplicateProfileName("focus".into()))
        );
    }

    #[test]
    fn only_one_manual_profile_is_active() {
        let mut state = State::fixture();
        let a = state
            .add_profile(Profile::new("A", t(8, 0)).unwrap())
            .unwrap();
        let b = state
            .add_profile(Profile::new("B", t(8, 0)).unwrap())
            .unwrap();
        state.activate_manual(a, t(8, 0)).unwrap();
        state.activate_manual(b, t(8, 1)).unwrap();
        assert_eq!(state.manual_active, Some(b));
        assert!(!state.profile_is_active(state.profile(a).unwrap(), t(9, 0)));
        assert!(state.profile_is_active(state.profile(b).unwrap(), t(9, 0)));
    }

    #[test]
    fn allowlist_entries_have_stable_rule_identity() {
        let mut rules = crate::domain::RuleSet::default();
        let host = HostPattern::new("example.com").unwrap();
        let first = rules.set_site_rule(host.clone(), SiteAction::Block);
        let second = rules.set_site_rule(host, SiteAction::Block);
        assert_eq!(first, second);
    }
}
