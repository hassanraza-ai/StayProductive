//! Stable reason codes.
//!
//! Codes are part of the test and log contract; the UI renders them into
//! sentences. They never change meaning. See `docs/spec/domain-model.md` §11
//! and the precedence table in `docs/spec/blocking-engine.md` §3.

use std::fmt;

use serde::{Deserialize, Serialize};

/// Why a decision was made.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum ReasonCode {
    /// An active commitment, or a strict-eligible profile window.
    Strict,
    /// A profile window is active and the profile blocks this target.
    Profile,
    /// The daily usage allowance is exhausted.
    LimitDaily,
    /// The hourly usage allowance is exhausted.
    LimitHourly,
    /// A launch-count allowance is exhausted.
    Launch,
    /// A pickup-count allowance is exhausted.
    Pickup,
    /// A wait timer is running.
    Wait,
    /// A keyword matched the URL or search text.
    Keyword,
    /// A site rule matched.
    Site,
    /// A content-surface rule matched.
    Surface,
    /// An ad-hoc quick block is active.
    Quick,
    /// An allowlist entry overrides a non-strict rule.
    Allow,
    /// The profile is paused or on a break.
    Pause,
    /// No rule applies; the default is to allow.
    NoRule,
}

impl ReasonCode {
    /// The stable wire/log code.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Strict => "R-STRICT",
            Self::Profile => "R-PROFILE",
            Self::LimitDaily => "R-LIMIT-DAILY",
            Self::LimitHourly => "R-LIMIT-HOURLY",
            Self::Launch => "R-LAUNCH",
            Self::Pickup => "R-PICKUP",
            Self::Wait => "R-WAIT",
            Self::Keyword => "R-KEYWORD",
            Self::Site => "R-SITE",
            Self::Surface => "R-SURFACE",
            Self::Quick => "R-QUICK",
            Self::Allow => "R-ALLOW",
            Self::Pause => "R-PAUSE",
            Self::NoRule => "R-NO-RULE",
        }
    }

    /// Whether this code describes a block.
    pub const fn is_block(self) -> bool {
        matches!(
            self,
            Self::Strict
                | Self::Profile
                | Self::LimitDaily
                | Self::LimitHourly
                | Self::Launch
                | Self::Pickup
                | Self::Wait
                | Self::Keyword
                | Self::Site
                | Self::Surface
                | Self::Quick
        )
    }

    /// Precedence used to pick one reason when several profiles block the same
    /// target. Lower is stronger.
    pub const fn precedence(self) -> u8 {
        match self {
            Self::Strict => 0,
            Self::Profile => 1,
            Self::Quick => 2,
            Self::Wait => 3,
            Self::LimitHourly => 4,
            Self::LimitDaily => 5,
            Self::Launch => 6,
            Self::Pickup => 7,
            Self::Keyword => 8,
            Self::Site => 9,
            Self::Surface => 10,
            Self::Allow => 90,
            Self::Pause => 91,
            Self::NoRule => 99,
        }
    }
}

impl fmt::Display for ReasonCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::ReasonCode;

    #[test]
    fn codes_are_stable() {
        assert_eq!(ReasonCode::Strict.as_str(), "R-STRICT");
        assert_eq!(ReasonCode::LimitDaily.as_str(), "R-LIMIT-DAILY");
        assert_eq!(ReasonCode::NoRule.as_str(), "R-NO-RULE");
    }

    #[test]
    fn strict_is_stronger_than_profile() {
        assert!(ReasonCode::Strict.precedence() < ReasonCode::Profile.precedence());
        assert!(ReasonCode::Profile.precedence() < ReasonCode::Site.precedence());
    }
}
