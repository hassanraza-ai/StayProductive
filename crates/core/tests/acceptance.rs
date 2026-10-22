//! Acceptance scenarios from `docs/acceptance/parity-tests.md`.
//!
//! Each test names its `A-*` identifier so the parity matrix can point at a real
//! executable test. These tests drive the pure core with a fake clock; no
//! desktop, database, or network is involved.

use std::time::Duration;

use chrono::{NaiveDate, NaiveTime, TimeZone, Utc, Weekday};
use stayproductive_core::{
    Activation, AppKey, AppRule, Commitment, CommitmentKind, CountLimit, Event, HostPattern,
    LimitPeriod, PauseReason, Profile, Schedule, State, Target, Timestamp, Verdict, Window,
    day_period, decide, logical_date,
};

fn at(year: i32, month: u32, day: u32, hour: u32, minute: u32, second: u32) -> Timestamp {
    Utc.with_ymd_and_hms(year, month, day, hour, minute, second)
        .unwrap()
}

/// Thursday 2026-10-22.
fn thu(hour: u32, minute: u32) -> Timestamp {
    at(2026, 10, 22, hour, minute, 0)
}

fn time(hour: u32, minute: u32) -> NaiveTime {
    NaiveTime::from_hms_opt(hour, minute, 0).unwrap()
}

fn window(start: (u32, u32), end: (u32, u32)) -> Window {
    Window::new(time(start.0, start.1), time(end.0, end.1))
}

fn fg(app: &str) -> Event {
    Event::ForegroundChanged {
        app: AppKey::new(app).unwrap(),
        window: None,
    }
}

fn url(app: &str, url: &str) -> Event {
    Event::UrlChanged {
        app: AppKey::new(app).unwrap(),
        url: url.to_owned(),
        title: None,
    }
}

fn always(state: &mut State, profile: Profile) -> stayproductive_core::ProfileId {
    let profile = profile.with_activation(Activation::Always);
    state.add_profile(profile).unwrap()
}

fn manual(state: &mut State, profile: Profile, now: Timestamp) -> stayproductive_core::ProfileId {
    let id = state.add_profile(profile).unwrap();
    state.activate_manual(id, now).unwrap();
    id
}

// ---------------------------------------------------------------- profiles

#[test]
fn a_prof_01_rename_duplicate_preserves_rules_without_duplicating_history() {
    let mut state = State::fixture();
    let original = manual(
        &mut state,
        Profile::new("Focus", thu(8, 0))
            .unwrap()
            .block_app("firefox")
            .unwrap()
            .daily_limit("social", Duration::from_secs(30 * 60))
            .unwrap(),
        thu(8, 0),
    );
    state
        .rename_profile(original, "Deep Work", thu(8, 5))
        .unwrap();
    let copy = state
        .duplicate_profile(original, "Deep Work Copy", thu(8, 6))
        .unwrap();

    assert_eq!(state.profile(original).unwrap().name, "Deep Work");
    assert_eq!(state.profile(copy).unwrap().name, "Deep Work Copy");
    assert_eq!(state.profile(copy).unwrap().rules.apps.len(), 2);
    // History is held once, globally, not attached to the duplicated profile.
    assert_eq!(state.usage.intervals().len(), 0);

    assert_eq!(
        state.rename_profile(copy, "Deep Work", thu(8, 7)),
        Err(stayproductive_core::Error::DuplicateProfileName(
            "Deep Work".into()
        ))
    );
    assert_eq!(
        state.duplicate_profile(original, "Deep Work", thu(8, 8)),
        Err(stayproductive_core::Error::DuplicateProfileName(
            "Deep Work".into()
        ))
    );
}

#[test]
fn a_prof_02_manual_activation_is_one_shared_state() {
    let mut state = State::fixture();
    let id = state
        .add_profile(
            Profile::new("Focus", thu(8, 0))
                .unwrap()
                .block_app("firefox")
                .unwrap(),
        )
        .unwrap();
    state.activate_manual(id, thu(8, 0)).unwrap();
    assert_eq!(state.manual_active, Some(id));
    assert!(state.profile_is_active(state.profile(id).unwrap(), thu(9, 0)));
    let decision = decide(&state, &fg("firefox"), thu(9, 0));
    assert_eq!(decision.reason, stayproductive_core::ReasonCode::Profile);
}

#[test]
fn a_prof_03_activating_b_deactivates_a() {
    let mut state = State::fixture();
    let a = state
        .add_profile(Profile::new("A", thu(8, 0)).unwrap())
        .unwrap();
    let b = state
        .add_profile(Profile::new("B", thu(8, 0)).unwrap())
        .unwrap();
    state.activate_manual(a, thu(8, 0)).unwrap();
    state.activate_manual(b, thu(8, 1)).unwrap();
    assert_eq!(state.manual_active, Some(b));
    assert!(!state.profile_is_active(state.profile(a).unwrap(), thu(9, 0)));
}

#[test]
fn a_prof_05_copy_items_adds_without_removing() {
    let mut state = State::fixture();
    let a = state
        .add_profile(
            Profile::new("A", thu(8, 0))
                .unwrap()
                .block_app("firefox")
                .unwrap(),
        )
        .unwrap();
    let b = state
        .add_profile(
            Profile::new("B", thu(8, 0))
                .unwrap()
                .block_app("editor")
                .unwrap(),
        )
        .unwrap();
    state.copy_items(a, b, thu(8, 10)).unwrap();
    let rules = &state.profile(b).unwrap().rules;
    assert!(rules.apps.contains_key(&AppKey::new("firefox").unwrap()));
    assert!(rules.apps.contains_key(&AppKey::new("editor").unwrap()));
}

#[test]
fn a_prof_06_strict_profile_blocks_even_when_another_is_paused() {
    let mut state = State::fixture();
    let other = always(
        &mut state,
        Profile::new("Other", thu(8, 0))
            .unwrap()
            .block_app("firefox")
            .unwrap(),
    );
    state
        .pause_profile(other, None, PauseReason::User, thu(9, 0))
        .unwrap();
    always(
        &mut state,
        Profile::new("Strict", thu(8, 0))
            .unwrap()
            .with_strict_eligible(true)
            .block_app("firefox")
            .unwrap(),
    );
    let decision = decide(&state, &fg("firefox"), thu(10, 0));
    assert_eq!(decision.reason, stayproductive_core::ReasonCode::Strict);
}

// ------------------------------------------------------------------- rules

#[test]
fn a_rule_01_blocked_app_reports_r_profile() {
    let mut state = State::fixture();
    manual(
        &mut state,
        Profile::new("Focus", thu(8, 0))
            .unwrap()
            .block_app("firefox")
            .unwrap(),
        thu(8, 0),
    );
    let decision = decide(&state, &fg("firefox"), thu(9, 30));
    assert_eq!(decision.verdict, Verdict::Block);
    assert_eq!(decision.reason, stayproductive_core::ReasonCode::Profile);
}

#[test]
fn a_rule_02_blocked_site_reports_r_site() {
    let mut state = State::fixture();
    manual(
        &mut state,
        Profile::new("Focus", thu(8, 0))
            .unwrap()
            .block_site("example.com")
            .unwrap(),
        thu(8, 0),
    );
    let decision = decide(
        &state,
        &url("firefox", "https://example.com/path"),
        thu(9, 30),
    );
    assert_eq!(decision.reason, stayproductive_core::ReasonCode::Site);
}

#[test]
fn a_rule_03_keyword_blocks_url_before_paint() {
    let mut state = State::fixture();
    manual(
        &mut state,
        Profile::new("Focus", thu(8, 0))
            .unwrap()
            .block_keyword("game")
            .unwrap(),
        thu(8, 0),
    );
    let decision = decide(
        &state,
        &url("firefox", "https://example.com/a/game/play"),
        thu(9, 30),
    );
    assert_eq!(decision.reason, stayproductive_core::ReasonCode::Keyword);
}

#[test]
fn a_rule_04_allowlist_overrides_a_non_strict_rule() {
    let mut state = State::fixture();
    manual(
        &mut state,
        Profile::new("Focus", thu(8, 0))
            .unwrap()
            .block_app("firefox")
            .unwrap()
            .allow_app("firefox")
            .unwrap(),
        thu(8, 0),
    );
    let decision = decide(&state, &fg("firefox"), thu(9, 30));
    assert_eq!(decision.verdict, Verdict::Allow);
    assert_eq!(decision.reason, stayproductive_core::ReasonCode::Allow);
}

#[test]
fn a_rule_05_wildcard_blocks_but_specific_subdomain_is_allowed() {
    let mut state = State::fixture();
    manual(
        &mut state,
        Profile::new("Focus", thu(8, 0))
            .unwrap()
            .block_site("*.example.com")
            .unwrap()
            .allow_site("good.example.com")
            .unwrap(),
        thu(8, 0),
    );
    assert_eq!(
        decide(
            &state,
            &url("firefox", "https://bad.example.com/"),
            thu(9, 0)
        )
        .reason,
        stayproductive_core::ReasonCode::Site
    );
    assert_eq!(
        decide(
            &state,
            &url("firefox", "https://good.example.com/"),
            thu(9, 0)
        )
        .reason,
        stayproductive_core::ReasonCode::Allow
    );
    // The wildcard does not cover the apex.
    assert_eq!(
        decide(&state, &url("firefox", "https://example.com/"), thu(9, 0)).reason,
        stayproductive_core::ReasonCode::NoRule
    );
}

#[test]
fn a_rule_07_wait_timer_denies_access_until_it_ends() {
    let mut state = State::fixture();
    always(
        &mut state,
        Profile::new("Focus", thu(8, 0))
            .unwrap()
            .wait_timer(
                "social",
                Duration::from_secs(25 * 60),
                Duration::from_secs(5 * 60),
                false,
            )
            .unwrap(),
    );
    state.observe(&fg("social"), thu(10, 0));
    state.observe(&fg("social"), thu(10, 25));
    let blocked = decide(&state, &fg("social"), thu(10, 25));
    assert_eq!(blocked.reason, stayproductive_core::ReasonCode::Wait);
    assert_eq!(blocked.until, Some(thu(10, 30)));
    assert_eq!(
        decide(&state, &fg("social"), thu(10, 30)).reason,
        stayproductive_core::ReasonCode::NoRule
    );
}

#[test]
fn a_rule_08_quick_block_expires_without_profile_edits() {
    let mut state = State::fixture();
    state
        .quick_block(
            Target::App(AppKey::new("firefox").unwrap()),
            thu(9, 10),
            thu(9, 0),
        )
        .unwrap();
    assert_eq!(
        decide(&state, &fg("firefox"), thu(9, 5)).reason,
        stayproductive_core::ReasonCode::Quick
    );
    assert_eq!(
        decide(&state, &fg("firefox"), thu(9, 10)).reason,
        stayproductive_core::ReasonCode::NoRule
    );
}

// ------------------------------------------------------------------ limits

#[test]
fn a_lim_01_daily_limit_blocks_at_exactly_the_limit_until_next_day() {
    let mut state = State::fixture();
    always(
        &mut state,
        Profile::new("Focus", thu(8, 0))
            .unwrap()
            .daily_limit("social", Duration::from_secs(30 * 60))
            .unwrap(),
    );
    state.observe(&fg("social"), thu(10, 0));
    state.observe(&fg("social"), thu(10, 30));
    let decision = decide(&state, &fg("social"), thu(10, 30));
    assert_eq!(decision.reason, stayproductive_core::ReasonCode::LimitDaily);
    assert_eq!(decision.until, Some(at(2026, 10, 23, 0, 0, 0)));
}

#[test]
fn a_lim_02_hourly_limit_blocks_until_the_next_hour() {
    let mut state = State::fixture();
    always(
        &mut state,
        Profile::new("Focus", thu(8, 0))
            .unwrap()
            .hourly_limit("social", Duration::from_secs(5 * 60))
            .unwrap(),
    );
    state.observe(&fg("social"), thu(10, 54));
    state.observe(&fg("social"), thu(10, 59));
    let decision = decide(&state, &fg("social"), thu(10, 59));
    assert_eq!(
        decision.reason,
        stayproductive_core::ReasonCode::LimitHourly
    );
    assert_eq!(decision.until, Some(thu(11, 0)));
    assert_eq!(
        decide(&state, &fg("social"), thu(11, 0)).reason,
        stayproductive_core::ReasonCode::NoRule
    );
}

#[test]
fn a_lim_03_launch_limit_ignores_quick_switches_and_blocks_the_third_launch() {
    let mut state = State::fixture();
    always(
        &mut state,
        Profile::new("Focus", thu(8, 0))
            .unwrap()
            .launch_limit("social", 2, LimitPeriod::Hourly)
            .unwrap(),
    );
    state.observe(&fg("social"), at(2026, 10, 22, 10, 0, 0));
    state.observe(&fg("editor"), at(2026, 10, 22, 10, 0, 1));
    state.observe(&fg("social"), at(2026, 10, 22, 10, 0, 2)); // within 5 s: not a launch
    assert!(!decide(&state, &fg("social"), at(2026, 10, 22, 10, 0, 2)).is_block());
    state.observe(&fg("editor"), at(2026, 10, 22, 10, 0, 7));
    state.observe(&fg("social"), at(2026, 10, 22, 10, 0, 13)); // > 5 s away: second launch
    assert!(!decide(&state, &fg("social"), at(2026, 10, 22, 10, 0, 13)).is_block());
    state.observe(&fg("editor"), at(2026, 10, 22, 10, 0, 19));
    state.observe(&fg("social"), at(2026, 10, 22, 10, 0, 26)); // third launch, blocked
    assert_eq!(
        decide(&state, &fg("social"), at(2026, 10, 22, 10, 0, 26)).reason,
        stayproductive_core::ReasonCode::Launch
    );
}

#[test]
fn a_lim_04_pickup_limit_reports_r_pickup() {
    let mut state = State::fixture();
    always(
        &mut state,
        Profile::new("Focus", thu(8, 0))
            .unwrap()
            .with_pickup_limit(20, LimitPeriod::Daily)
            .unwrap(),
    );
    for minute in 0..20 {
        state.observe(&Event::ScreenUnlocked, thu(9, minute));
    }
    let decision = decide(&state, &fg("social"), thu(9, 30));
    assert_eq!(decision.reason, stayproductive_core::ReasonCode::Pickup);
}

#[test]
fn a_lim_05_restart_resumes_the_counter_without_losing_time() {
    let mut state = State::fixture();
    always(
        &mut state,
        Profile::new("Focus", thu(8, 0))
            .unwrap()
            .daily_limit("social", Duration::from_secs(30 * 60))
            .unwrap(),
    );
    state.observe(&fg("social"), thu(10, 0));
    state.observe(&Event::Tick, at(2026, 10, 22, 10, 29, 59));

    let persisted = serde_json::to_string(&state).unwrap();
    let mut restored: State = serde_json::from_str(&persisted).unwrap();
    restored.recover();
    let app = AppKey::new("social").unwrap();
    let period = day_period(at(2026, 10, 22, 10, 30, 1), &restored.settings);
    assert_eq!(
        restored
            .usage
            .usage(&app, &period, at(2026, 10, 22, 10, 30, 1)),
        Duration::from_secs(29 * 60 + 59)
    );

    restored.observe(&fg("social"), at(2026, 10, 22, 10, 30, 1));
    let decision = decide(&restored, &fg("social"), at(2026, 10, 22, 10, 30, 2));
    assert_eq!(decision.reason, stayproductive_core::ReasonCode::LimitDaily);
}

#[test]
fn a_lim_06_start_of_day_counts_after_midnight_against_the_logical_day() {
    let settings =
        stayproductive_core::ClockSettings::with_start_of_day(time(4, 0), chrono_tz::UTC);
    let mut state = State::new(settings);
    always(
        &mut state,
        Profile::new("Focus", at(2026, 10, 19, 20, 0, 0))
            .unwrap()
            .daily_limit("social", Duration::from_secs(30 * 60))
            .unwrap(),
    );
    // Monday 23:00 -> 23:30 usage.
    state.observe(&fg("social"), at(2026, 10, 19, 23, 0, 0));
    state.observe(&fg("editor"), at(2026, 10, 19, 23, 30, 0));
    // Tuesday 02:00 still belongs to Monday.
    let decision = decide(&state, &fg("social"), at(2026, 10, 20, 2, 0, 0));
    assert_eq!(decision.reason, stayproductive_core::ReasonCode::LimitDaily);
    assert_eq!(decision.until, Some(at(2026, 10, 20, 4, 0, 0)));
    // The 04:00 boundary resets the counter.
    assert_eq!(
        decide(&state, &fg("social"), at(2026, 10, 20, 4, 0, 0)).reason,
        stayproductive_core::ReasonCode::NoRule
    );
    assert_eq!(
        logical_date(at(2026, 10, 20, 2, 0, 0), &state.settings),
        NaiveDate::from_ymd_opt(2026, 10, 19).unwrap()
    );
}

// --------------------------------------------------------------- schedules

#[test]
fn a_sch_01_daily_window_boundaries_are_half_open() {
    let mut state = State::fixture();
    state
        .add_profile(
            Profile::new("Focus", thu(8, 0))
                .unwrap()
                .with_activation(Activation::Scheduled)
                .with_schedule(Schedule::daily(vec![window((9, 0), (17, 0))]).unwrap())
                .block_app("firefox")
                .unwrap(),
        )
        .unwrap();
    assert!(!decide(&state, &fg("firefox"), at(2026, 10, 22, 8, 59, 59)).is_block());
    assert!(decide(&state, &fg("firefox"), at(2026, 10, 22, 9, 0, 0)).is_block());
    assert!(decide(&state, &fg("firefox"), at(2026, 10, 22, 16, 59, 59)).is_block());
    assert!(!decide(&state, &fg("firefox"), at(2026, 10, 22, 17, 0, 0)).is_block());
}

#[test]
fn a_sch_02_weekday_schedule_is_inactive_on_the_weekend() {
    let mut state = State::fixture();
    state
        .add_profile(
            Profile::new("Focus", thu(8, 0))
                .unwrap()
                .with_activation(Activation::Scheduled)
                .with_schedule(
                    Schedule::days(
                        stayproductive_core::DayPattern::weekdays(),
                        vec![window((8, 0), (17, 0))],
                    )
                    .unwrap(),
                )
                .block_app("firefox")
                .unwrap(),
        )
        .unwrap();
    assert!(decide(&state, &fg("firefox"), at(2026, 10, 23, 9, 0, 0)).is_block()); // Friday
    assert!(!decide(&state, &fg("firefox"), at(2026, 10, 24, 9, 0, 0)).is_block()); // Saturday
    assert!(decide(&state, &fg("firefox"), at(2026, 10, 26, 9, 0, 0)).is_block()); // Monday
}

#[test]
fn a_sch_03_multiple_intervals_allow_the_gap() {
    let mut state = State::fixture();
    state
        .add_profile(
            Profile::new("Focus", thu(8, 0))
                .unwrap()
                .with_activation(Activation::Scheduled)
                .with_schedule(
                    Schedule::intervals(vec![window((9, 0), (12, 0)), window((14, 0), (18, 0))])
                        .unwrap(),
                )
                .block_app("firefox")
                .unwrap(),
        )
        .unwrap();
    assert!(!decide(&state, &fg("firefox"), thu(12, 30)).is_block());
    assert!(!decide(&state, &fg("firefox"), thu(13, 59)).is_block());
    assert!(decide(&state, &fg("firefox"), thu(14, 0)).is_block());
}

#[test]
fn a_sch_04_full_day_and_one_time_windows() {
    let mut state = State::fixture();
    state
        .add_profile(
            Profile::new("Sunday", thu(8, 0))
                .unwrap()
                .with_activation(Activation::Scheduled)
                .with_schedule(
                    Schedule::full_day(stayproductive_core::DayPattern::day(Weekday::Sun)).unwrap(),
                )
                .block_app("firefox")
                .unwrap(),
        )
        .unwrap();
    assert!(decide(&state, &fg("firefox"), at(2026, 10, 25, 0, 0, 0)).is_block());
    assert!(decide(&state, &fg("firefox"), at(2026, 10, 25, 23, 59, 59)).is_block());
    assert!(!decide(&state, &fg("firefox"), at(2026, 10, 26, 0, 0, 0)).is_block());

    let range = stayproductive_core::DateRange::new(
        NaiveDate::from_ymd_opt(2026, 11, 3).unwrap(),
        NaiveDate::from_ymd_opt(2026, 11, 3).unwrap(),
    )
    .unwrap();
    state
        .add_profile(
            Profile::new("Exam", thu(8, 0))
                .unwrap()
                .with_activation(Activation::Scheduled)
                .with_schedule(Schedule::one_time(range, vec![window((9, 0), (17, 0))]).unwrap())
                .block_app("editor")
                .unwrap(),
        )
        .unwrap();
    assert!(!decide(&state, &fg("editor"), at(2026, 11, 2, 10, 0, 0)).is_block());
    assert!(decide(&state, &fg("editor"), at(2026, 11, 3, 10, 0, 0)).is_block());
    assert!(!decide(&state, &fg("editor"), at(2026, 11, 4, 10, 0, 0)).is_block());
}

#[test]
fn a_sch_05_hourly_pattern_uses_the_duty_cycle() {
    let mut state = State::fixture();
    state
        .add_profile(
            Profile::new("Focus", thu(8, 0))
                .unwrap()
                .with_activation(Activation::Scheduled)
                .with_schedule(
                    Schedule::hourly(
                        stayproductive_core::DayPattern::EveryDay,
                        stayproductive_core::HourlyPattern::fifty_ten(),
                    )
                    .unwrap(),
                )
                .block_app("firefox")
                .unwrap(),
        )
        .unwrap();
    assert!(decide(&state, &fg("firefox"), thu(10, 0)).is_block());
    assert!(decide(&state, &fg("firefox"), thu(10, 49)).is_block());
    assert!(!decide(&state, &fg("firefox"), thu(10, 50)).is_block());
    assert!(decide(&state, &fg("firefox"), thu(11, 0)).is_block());
}

#[test]
fn a_sch_06_daily_reset_happens_at_start_of_day_not_midnight() {
    let settings =
        stayproductive_core::ClockSettings::with_start_of_day(time(4, 0), chrono_tz::UTC);
    let mut state = State::new(settings);
    always(
        &mut state,
        Profile::new("Focus", at(2026, 10, 19, 20, 0, 0))
            .unwrap()
            .daily_limit("social", Duration::from_secs(30 * 60))
            .unwrap(),
    );
    state.observe(&fg("social"), at(2026, 10, 19, 23, 0, 0));
    state.observe(&fg("editor"), at(2026, 10, 19, 23, 30, 0));
    let before = day_period(at(2026, 10, 20, 3, 59, 59), &state.settings);
    let after = day_period(at(2026, 10, 20, 4, 0, 0), &state.settings);
    assert_eq!(before.end, at(2026, 10, 20, 4, 0, 0));
    assert_eq!(after.start, at(2026, 10, 20, 4, 0, 0));
    assert_eq!(
        decide(&state, &fg("social"), at(2026, 10, 20, 3, 59, 59)).reason,
        stayproductive_core::ReasonCode::LimitDaily
    );
    assert_eq!(
        decide(&state, &fg("social"), at(2026, 10, 20, 4, 0, 0)).reason,
        stayproductive_core::ReasonCode::NoRule
    );
}

#[test]
fn a_sch_07_pause_suppresses_then_resumes_at_the_deadline() {
    let mut state = State::fixture();
    let id = manual(
        &mut state,
        Profile::new("Focus", thu(8, 0))
            .unwrap()
            .block_app("firefox")
            .unwrap(),
        thu(8, 0),
    );
    state
        .pause_profile(id, Some(thu(9, 15)), PauseReason::User, thu(9, 0))
        .unwrap();
    assert_eq!(
        decide(&state, &fg("firefox"), thu(9, 5)).reason,
        stayproductive_core::ReasonCode::Pause
    );
    assert_eq!(
        decide(&state, &fg("firefox"), thu(9, 15)).reason,
        stayproductive_core::ReasonCode::Profile
    );
}

#[test]
fn a_sch_08_indefinite_pause_survives_restart() {
    let mut state = State::fixture();
    let id = manual(
        &mut state,
        Profile::new("Focus", thu(8, 0))
            .unwrap()
            .block_app("firefox")
            .unwrap(),
        thu(8, 0),
    );
    state
        .pause_profile(id, None, PauseReason::User, thu(9, 0))
        .unwrap();
    let persisted = serde_json::to_string(&state).unwrap();
    let restored: State = serde_json::from_str(&persisted).unwrap();
    assert_eq!(
        decide(&restored, &fg("firefox"), thu(23, 0)).reason,
        stayproductive_core::ReasonCode::Pause
    );
    let mut resumed = restored;
    resumed.resume_profile(id, thu(23, 1)).unwrap();
    assert_eq!(
        decide(&resumed, &fg("firefox"), thu(23, 2)).reason,
        stayproductive_core::ReasonCode::Profile
    );
}

#[test]
fn a_sch_09_extend_moves_the_end_forward_only() {
    let mut state = State::fixture();
    let id = state
        .add_profile(
            Profile::new("Focus", thu(8, 0))
                .unwrap()
                .with_activation(Activation::Scheduled)
                .with_schedule(Schedule::daily(vec![window((9, 0), (17, 0))]).unwrap())
                .block_app("firefox")
                .unwrap(),
        )
        .unwrap();
    let extended = state
        .extend_block(id, Duration::from_secs(30 * 60), thu(16, 0))
        .unwrap();
    assert_eq!(extended, thu(17, 30));
    let decision = decide(&state, &fg("firefox"), thu(17, 10));
    assert!(decision.is_block());
    assert_eq!(decision.until, Some(thu(17, 30)));
    assert!(!decide(&state, &fg("firefox"), thu(17, 30)).is_block());
}

#[test]
fn a_sch_10_editing_a_schedule_applies_without_restart() {
    let mut state = State::fixture();
    let id = state
        .add_profile(
            Profile::new("Focus", thu(8, 0))
                .unwrap()
                .with_activation(Activation::Scheduled)
                .with_schedule(Schedule::daily(vec![window((9, 0), (17, 0))]).unwrap())
                .block_app("firefox")
                .unwrap(),
        )
        .unwrap();
    assert!(!decide(&state, &fg("firefox"), thu(17, 30)).is_block());
    state.profile_mut(id).unwrap().schedule =
        Some(Schedule::daily(vec![window((10, 0), (18, 0))]).unwrap());
    assert!(decide(&state, &fg("firefox"), thu(17, 30)).is_block());
}

// -------------------------------------------------------------------- data

#[test]
fn a_data_02_backwards_clock_jump_does_not_double_count_or_shorten_a_commitment() {
    let mut state = State::fixture();
    manual(
        &mut state,
        Profile::new("Focus", thu(8, 0))
            .unwrap()
            .block_app("firefox")
            .unwrap(),
        thu(8, 0),
    );
    state
        .add_commitment(Commitment::new(
            CommitmentKind::Strict,
            thu(8, 0),
            Some(thu(18, 0)),
            stayproductive_core::DeactivationMethod::Cooldown(Duration::from_secs(600)),
        ))
        .unwrap();
    state.observe(&fg("social"), thu(10, 0));
    state.observe(&Event::Tick, thu(10, 5));
    state.observe(
        &Event::ClockJumped {
            old: thu(10, 5),
            new: thu(9, 5),
        },
        thu(9, 5),
    );
    state.observe(&fg("social"), thu(9, 5));
    state.observe(&Event::Tick, thu(9, 10));

    let app = AppKey::new("social").unwrap();
    let period = day_period(thu(9, 10), &state.settings);
    assert_eq!(
        state.usage.usage(&app, &period, thu(9, 10)),
        Duration::from_secs(10 * 60)
    );
    // The commitment keeps its absolute wall-clock end.
    assert!(decide(&state, &fg("firefox"), thu(9, 10)).is_block());
}

#[test]
fn a_con_06_keyword_folds_full_width_and_case() {
    let mut state = State::fixture();
    manual(
        &mut state,
        Profile::new("Focus", thu(8, 0))
            .unwrap()
            .block_keyword("GaMe")
            .unwrap(),
        thu(8, 0),
    );
    for candidate in ["game", "GAME", "ｇａｍｅ"] {
        let event = url("firefox", &format!("https://example.com/{candidate}"));
        assert_eq!(
            decide(&state, &event, thu(9, 0)).reason,
            stayproductive_core::ReasonCode::Keyword,
            "{candidate}"
        );
    }
}

#[test]
fn app_rule_launch_limit_can_be_constructed_directly() {
    let mut profile = Profile::new("Focus", thu(8, 0)).unwrap();
    let app = AppKey::new("social").unwrap();
    profile.rules.set_app_rule(
        app,
        AppRule::LaunchLimit {
            count: 2,
            per: LimitPeriod::Hourly,
        },
    );
    let _ = CountLimit::new(2, LimitPeriod::Hourly).unwrap();
    let _ = HostPattern::new("example.com").unwrap();
}
