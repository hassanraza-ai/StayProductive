//! Error type shared by the core.

use thiserror::Error;

use crate::ids::Ulid;

/// A validation or lookup failure.
///
/// Errors describe *what the caller supplied*, never the user's data, so they
/// are safe to log.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum Error {
    #[error("app key is empty")]
    EmptyAppKey,
    #[error("app key is too long: {0} bytes")]
    AppKeyTooLong(usize),
    #[error("app key contains a control character")]
    AppKeyControlCharacter,

    #[error("host is empty")]
    EmptyHost,
    #[error("host is not a valid DNS name: {0}")]
    InvalidHost(String),
    #[error("host pattern is empty")]
    EmptyHostPattern,
    #[error("host pattern is not valid: {0}")]
    InvalidHostPattern(String),

    #[error("keyword is empty")]
    EmptyKeyword,

    #[error("profile name is empty")]
    EmptyProfileName,
    #[error("profile name is longer than {} characters", 64)]
    ProfileNameTooLong,
    #[error("a profile named {0:?} already exists")]
    DuplicateProfileName(String),
    #[error("unknown profile: {0}")]
    UnknownProfile(Ulid),
    #[error("unknown commitment: {0}")]
    UnknownCommitment(Ulid),

    #[error("invalid schedule: {0}")]
    InvalidSchedule(String),
    #[error("invalid duration: {0}")]
    InvalidDuration(String),
    #[error("invalid rule: {0}")]
    InvalidRule(String),

    #[error("schedule is missing for a scheduled profile")]
    MissingSchedule,
    #[error("profile name must be unique")]
    DuplicateName,
    #[error("a commitment forbids this action")]
    CommitmentBlocksAction,
    #[error("commitment deactivation is not ready yet")]
    CommitmentNotReady,
    #[error("this commitment method is not supported yet")]
    UnsupportedCommitmentMethod,
}

/// Convenient result alias.
pub type Result<T> = std::result::Result<T, Error>;
