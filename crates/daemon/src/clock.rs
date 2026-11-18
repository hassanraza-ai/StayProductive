//! Wall-clock jumps compared with a monotonic clock.
//!
//! The daemon is the only place that reads the clock. A jump is a difference
//! between wall elapsed time and monotonic elapsed time, not a guess inside
//! the rule engine.

use std::time::{Duration, Instant};

use stayproductive_core::Timestamp;

/// Return `(previous wall, new wall)` when the wall clock moved by more than
/// `threshold` relative to the monotonic clock.
///
/// Drift at or under the threshold is ordinary scheduling delay and is not a
/// jump. A backwards step and a large forward step both qualify.
pub fn clock_discontinuity(
    previous_wall: Timestamp,
    previous_mono: Instant,
    wall: Timestamp,
    mono: Instant,
    threshold: Duration,
) -> Option<(Timestamp, Timestamp)> {
    let wall_ms = wall.signed_duration_since(previous_wall).num_milliseconds();
    let mono_ms = i64::try_from(mono.saturating_duration_since(previous_mono).as_millis())
        .unwrap_or(i64::MAX);
    let gap = wall_ms.abs_diff(mono_ms);
    if u128::from(gap) > threshold.as_millis() {
        Some((previous_wall, wall))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::clock_discontinuity;
    use chrono::{TimeZone, Utc};
    use std::time::{Duration, Instant};

    fn wall(seconds: i64) -> chrono::DateTime<Utc> {
        Utc.timestamp_opt(1_700_000_000 + seconds, 0).unwrap()
    }

    #[test]
    fn one_second_of_drift_is_not_a_jump() {
        let mono = Instant::now();
        assert!(
            clock_discontinuity(
                wall(0),
                mono,
                wall(1),
                mono + Duration::from_secs(1),
                Duration::from_secs(2),
            )
            .is_none()
        );
    }

    #[test]
    fn a_backwards_step_is_a_jump() {
        let mono = Instant::now();
        let jump = clock_discontinuity(
            wall(100),
            mono,
            wall(70),
            mono + Duration::from_secs(1),
            Duration::from_secs(2),
        );
        assert_eq!(jump, Some((wall(100), wall(70))));
    }

    #[test]
    fn a_forward_step_is_a_jump() {
        let mono = Instant::now();
        let jump = clock_discontinuity(
            wall(100),
            mono,
            wall(130),
            mono + Duration::from_secs(1),
            Duration::from_secs(2),
        );
        assert_eq!(jump, Some((wall(100), wall(130))));
    }
}
