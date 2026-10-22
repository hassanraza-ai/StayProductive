//! Stable keys used by rules, events and decisions.
//!
//! Keys are normalized once on construction so that comparisons are exact and
//! cheaper than re-normalizing on every decision. Display names are derived
//! elsewhere; a key is never a user-facing label.

use std::fmt;

use serde::{Deserialize, Serialize};
use unicode_normalization::UnicodeNormalization;

use crate::error::{Error, Result};

/// Maximum accepted app key length in bytes.
const MAX_APP_KEY: usize = 255;
/// Maximum accepted host length in bytes (DNS limit, without trailing dot).
const MAX_HOST: usize = 253;

/// Identity of a Linux application.
///
/// The canonical form is a lower-case desktop entry identifier
/// (`org.mozilla.firefox.desktop`) or, when no entry exists, the executable
/// basename (`firefox`). Rules reference this key, never a display name.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AppKey(String);

impl AppKey {
    /// Create a key, normalizing case and the optional `.desktop` suffix.
    pub fn new(value: impl AsRef<str>) -> Result<Self> {
        let value = value.as_ref().trim();
        if value.is_empty() {
            return Err(Error::EmptyAppKey);
        }
        if value.len() > MAX_APP_KEY {
            return Err(Error::AppKeyTooLong(value.len()));
        }
        if value.chars().any(char::is_control) || value.chars().any(char::is_whitespace) {
            return Err(Error::AppKeyControlCharacter);
        }
        let mut key = value.to_lowercase();
        if let Some(stripped) = key.strip_suffix(".desktop") {
            key.truncate(stripped.len());
        }
        if key.is_empty() {
            return Err(Error::EmptyAppKey);
        }
        Ok(Self(key))
    }

    /// The canonical key.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The desktop entry identifier, when the key came from one.
    pub fn desktop_id(&self) -> String {
        format!("{}.desktop", self.0)
    }
}

impl fmt::Display for AppKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl TryFrom<&str> for AppKey {
    type Error = Error;

    fn try_from(value: &str) -> Result<Self> {
        Self::new(value)
    }
}

/// A DNS host name, normalized to lower case and without a trailing dot.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Host(String);

impl Host {
    /// Parse and normalize a host.
    pub fn new(value: impl AsRef<str>) -> Result<Self> {
        let value = value.as_ref().trim();
        let value = value.strip_suffix('.').unwrap_or(value);
        if value.is_empty() {
            return Err(Error::EmptyHost);
        }
        if value.len() > MAX_HOST {
            return Err(Error::InvalidHost(value.to_owned()));
        }
        let host = value.to_lowercase();
        if !is_valid_host(&host) {
            return Err(Error::InvalidHost(value.to_owned()));
        }
        Ok(Self(host))
    }

    /// The normalized host.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Whether this host is a subdomain of `parent` (but not `parent` itself).
    pub fn is_subdomain_of(&self, parent: &Host) -> bool {
        self.0.len() > parent.0.len()
            && self.0.ends_with(&parent.0)
            && self.0.as_bytes()[self.0.len() - parent.0.len() - 1] == b'.'
    }

    /// Extract the host from an HTTP(S) URL.
    ///
    /// The extension sends full URLs; the core only needs the host. User info,
    /// ports, fragments, and non-HTTP schemes are ignored or rejected rather
    /// than guessed at.
    pub fn from_url(url: &str) -> Option<Self> {
        let rest = url.trim();
        let rest = rest
            .strip_prefix("https://")
            .or_else(|| rest.strip_prefix("http://"))
            .or_else(|| rest.strip_prefix("HTTPS://"))
            .or_else(|| rest.strip_prefix("HTTP://"))?;
        let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
        let authority = authority.rsplit('@').next().unwrap_or(authority);
        let host = if let Some(stripped) = authority.strip_prefix('[') {
            stripped.split(']').next().unwrap_or_default()
        } else {
            authority.split(':').next().unwrap_or_default()
        };
        Self::new(host).ok()
    }
}

impl fmt::Display for Host {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl TryFrom<&str> for Host {
    type Error = Error;

    fn try_from(value: &str) -> Result<Self> {
        Self::new(value)
    }
}

fn is_valid_host(host: &str) -> bool {
    if host.is_empty() || host.starts_with('.') || host.contains("..") {
        return false;
    }
    host.split('.').all(|label| {
        !label.is_empty()
            && label.len() <= 63
            && !label.starts_with('-')
            && !label.ends_with('-')
            && label
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
    })
}

/// A site rule pattern: an exact host or a `*.suffix` wildcard.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum HostPattern {
    /// Matches exactly this host.
    Exact(Host),
    /// Matches subdomains of this host, but not the host itself.
    Suffix(Host),
}

impl HostPattern {
    /// Parse `example.com` or `*.example.com`.
    pub fn new(value: impl AsRef<str>) -> Result<Self> {
        let value = value.as_ref().trim();
        if value.is_empty() {
            return Err(Error::EmptyHostPattern);
        }
        if let Some(rest) = value.strip_prefix("*.") {
            if rest.is_empty() {
                return Err(Error::InvalidHostPattern(value.to_owned()));
            }
            Ok(Self::Suffix(Host::new(rest)?))
        } else {
            Ok(Self::Exact(Host::new(value)?))
        }
    }

    /// Whether `host` matches this pattern.
    pub fn matches(&self, host: &Host) -> bool {
        match self {
            Self::Exact(expected) => expected == host,
            Self::Suffix(suffix) => host.is_subdomain_of(suffix),
        }
    }

    /// The host part, used for explanations.
    pub fn host(&self) -> &Host {
        match self {
            Self::Exact(host) | Self::Suffix(host) => host,
        }
    }
}

impl fmt::Display for HostPattern {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Exact(host) => host.fmt(f),
            Self::Suffix(host) => write!(f, "*.{host}"),
        }
    }
}

/// Where a keyword is matched.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum KeywordScope {
    /// The full URL.
    Url,
    /// The search/query component only.
    Search,
    /// Both of the above.
    #[default]
    Both,
}

/// A case-insensitive, Unicode-folded keyword.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Keyword(String);

impl Keyword {
    /// Normalize a keyword for matching.
    ///
    /// NFKC folds full-width forms (`ＧＡＭＥ` -> `GAME`); lowercasing then
    /// makes the comparison case-insensitive.
    pub fn new(value: impl AsRef<str>) -> Result<Self> {
        let value = value.as_ref().trim();
        if value.is_empty() {
            return Err(Error::EmptyKeyword);
        }
        Ok(Self(normalize_keyword(value)))
    }

    /// The normalized text.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Whether normalized `haystack` contains this keyword.
    pub fn matches(&self, haystack: &str) -> bool {
        normalize_keyword(haystack).contains(&self.0)
    }
}

impl fmt::Display for Keyword {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Normalize text for keyword comparison.
pub fn normalize_keyword(value: &str) -> String {
    value
        .nfkc()
        .flat_map(char::to_lowercase)
        .filter(|c| *c != '\u{200b}' && *c != '\u{200c}' && *c != '\u{200d}')
        .collect()
}

/// Identity of an in-page content surface (Shorts, Reels, Stories, …).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SurfaceId(String);

impl SurfaceId {
    /// Create a surface identifier.
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// The raw identifier.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for SurfaceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// An opaque compositor window identifier.
///
/// Window keys are ephemeral: they are used for enforcement and logs, never
/// exported by default and never used to identify a rule.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct WindowKey(String);

impl WindowKey {
    /// Create a window key.
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// The raw key.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for WindowKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_keys_are_case_folded_and_stripped() {
        assert_eq!(AppKey::new("Firefox.Desktop").unwrap().as_str(), "firefox");
        assert_eq!(
            AppKey::new("org.mozilla.Firefox.desktop").unwrap().as_str(),
            "org.mozilla.firefox"
        );
        assert_eq!(AppKey::new("firefox.desktop").unwrap().as_str(), "firefox");
    }

    #[test]
    fn app_keys_reject_junk() {
        assert_eq!(AppKey::new("   ").unwrap_err(), Error::EmptyAppKey);
        assert_eq!(
            AppKey::new("two words").unwrap_err(),
            Error::AppKeyControlCharacter
        );
    }

    #[test]
    fn host_patterns_match_subdomains_only() {
        let wildcard = HostPattern::new("*.example.com").unwrap();
        assert!(wildcard.matches(&Host::new("www.example.com").unwrap()));
        assert!(wildcard.matches(&Host::new("a.b.example.com").unwrap()));
        assert!(!wildcard.matches(&Host::new("example.com").unwrap()));
        assert!(!wildcard.matches(&Host::new("notexample.com").unwrap()));
        assert!(!wildcard.matches(&Host::new("example.com.evil").unwrap()));
    }

    #[test]
    fn exact_hosts_ignore_case_and_trailing_dot() {
        let exact = HostPattern::new("Example.COM.").unwrap();
        assert!(exact.matches(&Host::new("example.com").unwrap()));
        assert!(!exact.matches(&Host::new("www.example.com").unwrap()));
    }

    #[test]
    fn keyword_matching_folds_unicode_and_case() {
        let keyword = Keyword::new("GaMe").unwrap();
        assert!(keyword.matches("https://example.com/game"));
        assert!(keyword.matches("GAME"));
        assert!(keyword.matches("ｇａｍｅ"));
        assert!(!keyword.matches("gaming"));
    }

    #[test]
    fn invalid_hosts_are_rejected() {
        for bad in [
            "",
            "-bad.com",
            "bad-.com",
            "a..b",
            "spaces .com",
            "http://x",
        ] {
            assert!(Host::new(bad).is_err(), "{bad:?} should be rejected");
        }
    }
}
