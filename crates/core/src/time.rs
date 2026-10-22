//! Time primitives.
//!
//! The core never reads the wall clock. [`Timestamp`] is a UTC instant supplied
//! by the caller; local-time math goes through [`crate::period`] and
//! [`crate::schedule`].

use chrono::{DateTime, Utc};

/// A UTC instant.
///
/// Named `Timestamp` to avoid confusion with [`std::time::Instant`], which is
/// used for monotonic timers and must never be compared with this type.
pub type Timestamp = DateTime<Utc>;
