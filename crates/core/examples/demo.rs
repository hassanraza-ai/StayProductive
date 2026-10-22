//! A scripted walkthrough of the M0 core engine.
//!
//! Run with:
//!
//! ```sh
//! cargo run -p stayproductive-core --example demo
//! ```
//!
//! This is a console demonstration of the pure decision engine. There is no
//! daemon, compositor adapter, or GTK UI yet, so nothing is actually enforced
//! on the desktop: every observation below is fed to `State::observe` and the
//! resulting `Decision` is printed.

use std::time::Duration;

use chrono::{NaiveTime, TimeZone, Utc};
use stayproductive_core::{
    Activation, AppKey, ClockSettings, Commitment, CommitmentKind, DeactivationMethod, Event,
    Profile, Schedule, State, Timestamp, Verdict, Window, decide,
};

/// Thursday 2026-10-22.
fn thu(hour: u32, minute: u32) -> Timestamp {
    Utc.with_ymd_and_hms(2026, 10, 22, hour, minute, 0).unwrap()
}

fn time(hour: u32, minute: u32) -> NaiveTime {
    NaiveTime::from_hms_opt(hour, minute, 0).unwrap()
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

fn main() {
    println!("StayProductive core — M0 decision demo");
    println!("Clock is fixed at Thursday 2026-10-22 (UTC). No enforcement is active.\n");

    let mut state = State::new(ClockSettings::default());

    let focus = state
        .add_profile(
            Profile::new("Focus Hours", thu(8, 0))
                .unwrap()
                .with_activation(Activation::Scheduled)
                .with_schedule(
                    Schedule::days(
                        stayproductive_core::DayPattern::weekdays(),
                        vec![Window::new(time(9, 0), time(17, 0))],
                    )
                    .unwrap(),
                )
                .block_app("firefox")
                .unwrap()
                .block_site("*.example.com")
                .unwrap()
                .allow_site("good.example.com")
                .unwrap()
                .block_keyword("shorts")
                .unwrap()
                .daily_limit("social", Duration::from_secs(30 * 60))
                .unwrap(),
        )
        .unwrap();

    state
        .add_profile(
            Profile::new("Lockdown", thu(8, 0))
                .unwrap()
                .with_activation(Activation::Always)
                .with_strict_eligible(true)
                .block_app("editor")
                .unwrap(),
        )
        .unwrap();

    println!("Profiles:");
    for profile in &state.profiles {
        let kind = match profile.activation {
            Activation::Manual => "manual",
            Activation::Scheduled => "scheduled",
            Activation::Always => "always",
        };
        println!(
            "  - {} ({kind}{})",
            profile.name,
            if profile.strict_eligible {
                ", strict-eligible"
            } else {
                ""
            }
        );
    }
    println!();

    section("1. Schedule boundaries");
    scenario(&mut state, "08:59  firefox", thu(8, 59), fg("firefox"));
    scenario(&mut state, "09:00  firefox", thu(9, 0), fg("firefox"));

    section("2. Keyword and site rules");
    scenario(
        &mut state,
        "09:30  youtube.com/shorts",
        thu(9, 30),
        url("firefox", "https://youtube.com/shorts/abc"),
    );
    scenario(
        &mut state,
        "09:30  bad.example.com",
        thu(9, 30),
        url("firefox", "https://bad.example.com/page"),
    );
    scenario(
        &mut state,
        "09:30  good.example.com",
        thu(9, 30),
        url("firefox", "https://good.example.com/page"),
    );

    section("3. Daily usage limit (30 min on social)");
    state.observe(&fg("social"), thu(10, 0));
    println!("  10:00  social starts");
    state.observe(&fg("social"), thu(10, 30));
    scenario(&mut state, "10:30  social", thu(10, 30), fg("social"));

    section("4. Pause suppresses the profile, then expires");
    state
        .pause_profile(
            focus,
            Some(thu(11, 15)),
            stayproductive_core::PauseReason::User,
            thu(11, 0),
        )
        .unwrap();
    scenario(
        &mut state,
        "11:05  firefox (paused)",
        thu(11, 5),
        fg("firefox"),
    );
    scenario(
        &mut state,
        "11:15  firefox (resumed)",
        thu(11, 15),
        fg("firefox"),
    );

    section("5. Strict-eligible profile beats a pause elsewhere");
    scenario(&mut state, "11:20  editor", thu(11, 20), fg("editor"));

    section("6. Quick block with a visible end");
    state
        .quick_block(
            stayproductive_core::Target::App(AppKey::new("gimp").unwrap()),
            thu(12, 10),
            thu(12, 0),
        )
        .unwrap();
    scenario(&mut state, "12:05  gimp", thu(12, 5), fg("gimp"));
    scenario(&mut state, "12:10  gimp", thu(12, 10), fg("gimp"));

    section("7. Commitment cannot be overridden");
    state
        .add_commitment(Commitment::new(
            CommitmentKind::Strict,
            thu(12, 0),
            Some(thu(18, 0)),
            DeactivationMethod::Cooldown(Duration::from_secs(10 * 60)),
        ))
        .unwrap();
    state
        .pause_profile(
            focus,
            Some(thu(13, 0)),
            stayproductive_core::PauseReason::User,
            thu(12, 30),
        )
        .unwrap();
    scenario(
        &mut state,
        "12:35  firefox (paused + strict)",
        thu(12, 35),
        fg("firefox"),
    );

    println!("\nDone. The engine returned a reason and end time for every decision.");
    println!("Next: SQLite persistence, daemon/D-Bus, and a real X11 adapter.");
}

fn section(title: &str) {
    println!("{title}");
}

fn scenario(state: &mut State, label: &str, now: Timestamp, event: Event) {
    state.observe(&event, now);
    let decision = decide(state, &event, now);
    let profile = decision
        .profile
        .and_then(|id| state.profile(id))
        .map(|profile| profile.name.as_str())
        .unwrap_or("—");
    let until = decision
        .until
        .map(|until| {
            if until.date_naive() == now.date_naive() {
                until.format("%H:%M").to_string()
            } else {
                until.format("%a %H:%M").to_string()
            }
        })
        .unwrap_or_else(|| "—".to_owned());
    let verdict = match decision.verdict {
        Verdict::Allow => "ALLOW",
        Verdict::Block => "BLOCK",
        Verdict::BlockUntil => "BLOCK",
        Verdict::AllowWithNotice => "ALLOW*",
    };
    println!(
        "  {label:<32} {verdict:<6} {:<15} until {:<10} {profile}",
        decision.reason.as_str(),
        until
    );
    if decision.is_block() {
        println!("  {:<32} message: {}", "", decision.message);
    }
}
