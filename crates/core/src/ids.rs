//! Identifier types.
//!
//! Every persistent entity gets a ULID. Identity is stable across renames and
//! deletions (see `docs/spec/domain-model.md` §1.4).

pub use ulid::Ulid;

/// A profile identifier.
pub type ProfileId = Ulid;
/// A rule identifier.
pub type RuleId = Ulid;
/// A commitment identifier.
pub type CommitmentId = Ulid;
/// A break or session identifier.
pub type SessionId = Ulid;

/// Generate a new identifier.
///
/// Callers that need reproducible tests can construct entities with an explicit
/// identifier through the `with_id` builders.
pub fn new_id() -> Ulid {
    Ulid::new()
}
