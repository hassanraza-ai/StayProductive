//! Versioned export of everything the user owns.
//!
//! The archive shape follows `docs/spec/domain-model.md` §13: a manifest that
//! names the schema version, profiles with their rules and schedules, settings,
//! commitments, and (when the user asks for it) monthly usage aggregates. The
//! zip is written with stored entries and no extra fields so the output is
//! deterministic and readable without a compression dependency.
//!
//! Nothing here reads the clock; the caller passes `now` so an export is a pure
//! function of `(state, now, include_usage)` and tests can assert exact bytes.

use std::collections::BTreeMap;

use chrono::{DateTime, Datelike, NaiveTime, Timelike, Utc};
use chrono_tz::Tz;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use stayproductive_core::{
    AppKey, ClockSettings, Commitment, Profile, ProfileId, QuickBlock, State, Target, Timestamp,
};

/// Schema version written into `manifest.json`.
///
/// Bump this only for a breaking change to the archive layout; imports must
/// refuse a version they do not understand rather than guess.
pub const EXPORT_VERSION: u32 = 1;

/// Marker that identifies our archive without looking at the file extension.
pub const EXPORT_FORMAT: &str = "stayproductive.export";

/// Build the uncompressed archive entries in a stable order.
///
/// History is opt-in: a backup of profiles and commitments does not need usage
/// hours, and the UI says which one it is exporting.
pub fn entries(state: &State, now: Timestamp, include_usage: bool) -> Vec<(String, Vec<u8>)> {
    let mut files: Vec<(String, Vec<u8>)> = Vec::new();
    let manifest = json!({
        "format": EXPORT_FORMAT,
        "version": EXPORT_VERSION,
        "exported_at": now,
        "app_version": env!("CARGO_PKG_VERSION"),
        "usage_included": include_usage,
    });
    files.push(("manifest.json".to_owned(), encode(&manifest)));

    let profiles = json!({
        "active": state.manual_active,
        "profiles": state.profiles,
        "quick_blocks": state.quick_blocks,
        "extensions": state.extensions,
    });
    files.push(("profiles.json".to_owned(), encode(&profiles)));

    let settings = json!({
        "start_of_day": state.settings.start_of_day.format("%H:%M").to_string(),
        "timezone": state.settings.timezone.name(),
    });
    files.push(("settings.json".to_owned(), encode(&settings)));

    let commitments = json!({
        "all": state.commitments,
    });
    files.push(("commitments.json".to_owned(), encode(&commitments)));

    if include_usage {
        files.extend(usage_entries(state, now));
    }
    files
}

/// Aggregate the ledger into `usage/YYYY-MM.json`, one file per month.
///
/// Events are grouped by their own month in UTC, matching how the ledger
/// timestamps them. Empty months are omitted so a fresh install exports no
/// usage files at all.
pub fn usage_entries(state: &State, now: Timestamp) -> Vec<(String, Vec<u8>)> {
    let mut months: BTreeMap<String, MonthUsage> = BTreeMap::new();
    let usage = &state.usage;
    for interval in usage.intervals() {
        push_interval(&mut months, &interval.app, interval.start, interval.end);
    }
    // The foreground app that is running right now is not a closed interval
    // yet; exporting only closed ones would silently drop the active session.
    if let (Some(app), Some(start)) = (usage.current_app(), usage.current_since()) {
        push_interval(&mut months, app, start, now.max(start));
    }
    for launch in usage.launches() {
        months
            .entry(month(launch.at))
            .or_default()
            .launches
            .push(json!({"app": launch.app.as_str(), "at": launch.at}));
    }
    for pickup in usage.pickups() {
        months
            .entry(month(*pickup))
            .or_default()
            .pickups
            .push(json!({"at": pickup}));
    }
    for block in usage.blocks() {
        let (kind, target) = match &block.target {
            Target::App(app) => ("app", app.as_str().to_owned()),
            Target::Site(host) => ("site", host.as_str().to_owned()),
        };
        months
            .entry(month(block.at))
            .or_default()
            .blocks
            .push(json!({
                "at": block.at,
                "kind": kind,
                "target": target,
                "reason": block.reason.as_str(),
                "until": block.until,
                "profile": block.profile.map(|id| id.to_string()),
            }));
    }
    months
        .into_iter()
        .map(|(key, month)| {
            let body = json!({
                "month": key,
                "intervals": month.intervals,
                "launches": month.launches,
                "pickups": month.pickups,
                "blocks": month.blocks,
            });
            (format!("usage/{key}.json"), encode(&body))
        })
        .collect()
}

#[derive(Default)]
struct MonthUsage {
    intervals: Vec<Value>,
    launches: Vec<Value>,
    pickups: Vec<Value>,
    blocks: Vec<Value>,
}

fn push_interval(
    months: &mut BTreeMap<String, MonthUsage>,
    app: &AppKey,
    start: Timestamp,
    end: Timestamp,
) {
    months
        .entry(month(start))
        .or_default()
        .intervals
        .push(json!({
            "app": app.as_str(),
            "start": start,
            "seconds": (end - start).num_seconds().max(0),
            "end": end,
        }));
}

fn month(at: Timestamp) -> String {
    format!("{:04}-{:02}", at.year(), at.month())
}

/// What an import did, so the UI can report it instead of guessing.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct ImportSummary {
    /// The export replaced an empty state instead of merging.
    pub replaced: bool,
    pub profiles_added: usize,
    pub profiles_skipped: usize,
    pub commitments_added: usize,
    pub commitments_skipped: usize,
    pub settings_applied: bool,
}

/// Apply an export to `state`.
///
/// An empty state is replaced wholesale: profiles, manual activation, quick
/// blocks, schedule extensions, settings, and commitments. A state that already
/// has profiles or commitments is merged additively: unknown profiles are added
/// by identity, settings are left alone, and commitments are only added when no
/// commitment is enforcing right now. That is the "never weaken it" rule from
/// `A-UX-12`: an import can add strict time, never shorten or remove it.
///
/// Usage files are validated when present but not re-imported; the counters
/// have no restore path yet, and a silently partial history would be worse than
/// none. This is stated in the response and the return summary has no usage
/// count on purpose.
pub fn import(
    state: &mut State,
    files: &BTreeMap<String, Vec<u8>>,
    now: Timestamp,
) -> Result<ImportSummary, String> {
    let manifest: Value = parse_file(files, "manifest.json")?;
    if manifest.get("format").and_then(Value::as_str) != Some(EXPORT_FORMAT) {
        return Err("not a StayProductive export".into());
    }
    let version = manifest.get("version").and_then(Value::as_u64).unwrap_or(0);
    if version == 0 || version > u64::from(EXPORT_VERSION) {
        return Err(format!(
            "export version {version} is not supported by this build (supports {EXPORT_VERSION})"
        ));
    }
    let profiles: Vec<Profile> = parse_list(files, "profiles.json", "profiles")?;
    let commitments: Vec<Commitment> = parse_list(files, "commitments.json", "all")?;

    let clean = state.profiles.is_empty() && state.commitments.is_empty();
    let mut summary = ImportSummary {
        replaced: clean,
        ..ImportSummary::default()
    };

    if clean {
        if let Some(settings) = files.get("settings.json") {
            let settings: Value = serde_json::from_slice(settings)
                .map_err(|e| format!("invalid settings.json: {e}"))?;
            state.settings = clock_settings(&settings)?;
            summary.settings_applied = true;
        }
        for profile in profiles {
            state
                .add_profile(profile)
                .map_err(|e| format!("invalid profile in export: {e}"))?;
            summary.profiles_added += 1;
        }
        if let Some(value) = files
            .get("profiles.json")
            .and_then(|bytes| serde_json::from_slice::<Value>(bytes).ok())
        {
            if let Some(active) = value.get("active").and_then(Value::as_str)
                && let Ok(id) = active.parse::<ProfileId>()
                && state.profiles.iter().any(|profile| profile.id == id)
            {
                state.manual_active = Some(id);
            }
            if let Some(blocks) = value.get("quick_blocks")
                && let Ok(blocks) = serde_json::from_value::<Vec<QuickBlock>>(blocks.clone())
            {
                state.quick_blocks = blocks;
            }
            if let Some(extensions) = value.get("extensions")
                && let Ok(extensions) =
                    serde_json::from_value::<BTreeMap<ProfileId, Timestamp>>(extensions.clone())
            {
                state.extensions = extensions;
            }
        }
        for commitment in commitments {
            state
                .add_commitment(commitment)
                .map_err(|e| format!("invalid commitment in export: {e}"))?;
            summary.commitments_added += 1;
        }
        return Ok(summary);
    }

    for profile in profiles {
        if state.profiles.iter().any(|known| known.id == profile.id)
            || state.profile_by_name(&profile.name).is_some()
        {
            summary.profiles_skipped += 1;
            continue;
        }
        state
            .add_profile(profile)
            .map_err(|e| format!("invalid profile in export: {e}"))?;
        summary.profiles_added += 1;
    }
    let enforcing = state.active_commitment(now).is_some();
    for commitment in commitments {
        if enforcing
            || state
                .commitments
                .iter()
                .any(|known| known.id == commitment.id)
        {
            summary.commitments_skipped += 1;
            continue;
        }
        state
            .add_commitment(commitment)
            .map_err(|e| format!("invalid commitment in export: {e}"))?;
        summary.commitments_added += 1;
    }
    Ok(summary)
}

fn clock_settings(settings: &Value) -> Result<ClockSettings, String> {
    let start = settings
        .get("start_of_day")
        .and_then(Value::as_str)
        .ok_or_else(|| "settings.json is missing start_of_day".to_owned())?;
    let start_of_day = NaiveTime::parse_from_str(start, "%H:%M")
        .map_err(|_| format!("invalid start_of_day: {start}"))?;
    let timezone = settings
        .get("timezone")
        .and_then(Value::as_str)
        .unwrap_or("UTC");
    let timezone = timezone
        .parse::<Tz>()
        .map_err(|_| format!("unknown time zone: {timezone}"))?;
    Ok(ClockSettings {
        start_of_day,
        timezone,
    })
}

fn parse_file<T: DeserializeOwned>(
    files: &BTreeMap<String, Vec<u8>>,
    name: &str,
) -> Result<T, String> {
    let bytes = files
        .get(name)
        .ok_or_else(|| format!("archive is missing {name}"))?;
    serde_json::from_slice(bytes).map_err(|e| format!("invalid {name}: {e}"))
}

fn parse_list<T: DeserializeOwned>(
    files: &BTreeMap<String, Vec<u8>>,
    name: &str,
    key: &str,
) -> Result<Vec<T>, String> {
    let value: Value = parse_file(files, name)?;
    let list = value
        .get(key)
        .cloned()
        .unwrap_or_else(|| Value::Array(Vec::new()));
    serde_json::from_value(list).map_err(|e| format!("invalid {name}: {e}"))
}

/// Read a stored zip into memory.
///
/// Only method 0 entries are accepted: our export never compresses, and
/// refusing deflate is better than silently treating compressed bytes as
/// content. Bounds are checked before slicing so a truncated or hand-edited
/// file returns an error instead of panicking.
pub fn read_archive(bytes: &[u8]) -> Result<BTreeMap<String, Vec<u8>>, String> {
    if bytes.len() > 64 * 1024 * 1024 {
        return Err("export archive is too large".into());
    }
    let eocd = (0..bytes.len().saturating_sub(4))
        .rev()
        .find(|&at| &bytes[at..at + 4] == b"PK\x05\x06")
        .ok_or_else(|| "not a zip archive".to_owned())?;
    let count = read_u16(bytes, eocd + 10)? as usize;
    if count > 1024 {
        return Err("export archive has too many entries".into());
    }
    let mut at = read_u32(bytes, eocd + 16)? as usize;
    let mut files = BTreeMap::new();
    for _ in 0..count {
        if bytes.get(at..at + 4) != Some(b"PK\x01\x02") {
            return Err("damaged zip central directory".into());
        }
        let method = read_u16(bytes, at + 10)?;
        if method != 0 {
            return Err("compressed export entries are not supported".into());
        }
        let crc = read_u32(bytes, at + 16)?;
        let size = read_u32(bytes, at + 20)? as usize;
        let name_len = read_u16(bytes, at + 28)? as usize;
        let extra_len = read_u16(bytes, at + 30)? as usize;
        let comment_len = read_u16(bytes, at + 32)? as usize;
        let local = read_u32(bytes, at + 42)? as usize;
        let name = std::str::from_utf8(read_slice(bytes, at + 46, name_len)?)
            .map_err(|_| "zip entry name is not UTF-8".to_owned())?
            .to_owned();
        if bytes.get(local..local + 4) != Some(b"PK\x03\x04") {
            return Err(format!("damaged zip local header for {name}"));
        }
        let local_name = read_u16(bytes, local + 26)? as usize;
        let local_extra = read_u16(bytes, local + 28)? as usize;
        let start = local + 30 + local_name + local_extra;
        let data = read_slice(bytes, start, size)?;
        if crc32(data) != crc {
            return Err(format!("checksum mismatch for {name}"));
        }
        files.insert(name, data.to_vec());
        at += 46 + name_len + extra_len + comment_len;
    }
    Ok(files)
}

fn read_u16(bytes: &[u8], at: usize) -> Result<u16, String> {
    let slice = bytes
        .get(at..at + 2)
        .ok_or_else(|| "truncated zip archive".to_owned())?;
    Ok(u16::from_le_bytes([slice[0], slice[1]]))
}

fn read_u32(bytes: &[u8], at: usize) -> Result<u32, String> {
    let slice = bytes
        .get(at..at + 4)
        .ok_or_else(|| "truncated zip archive".to_owned())?;
    Ok(u32::from_le_bytes([slice[0], slice[1], slice[2], slice[3]]))
}

fn read_slice(bytes: &[u8], at: usize, len: usize) -> Result<&[u8], String> {
    bytes
        .get(at..at + len)
        .ok_or_else(|| "truncated zip archive".to_owned())
}

fn encode(value: &Value) -> Vec<u8> {
    let mut bytes = serde_json::to_vec_pretty(value).unwrap_or_default();
    bytes.push(b'\n');
    bytes
}

/// Pack stored entries into a zip archive.
///
/// Entries are uncompressed (method 0) and dates are fixed, so the same state
/// and clock produce the same bytes. The returned buffer is the whole file.
pub fn archive(files: &[(String, Vec<u8>)], now: Timestamp) -> Vec<u8> {
    let (dos_time, dos_date) = dos_datetime(now);
    let mut out: Vec<u8> = Vec::new();
    let mut central: Vec<u8> = Vec::new();

    for (name, data) in files {
        let offset = out.len() as u32;
        let crc = crc32(data);
        let name = name.as_bytes();
        let name_len = u16::try_from(name.len()).expect("archive entry name is short");
        let size = u32::try_from(data.len()).expect("archive entry fits in 4 GiB");

        put_u32(&mut out, 0x0403_4b50); // local file header
        put_u16(&mut out, 20); // version needed
        put_u16(&mut out, 0x0800); // UTF-8 names
        put_u16(&mut out, 0); // stored
        put_u16(&mut out, dos_time);
        put_u16(&mut out, dos_date);
        put_u32(&mut out, crc);
        put_u32(&mut out, size);
        put_u32(&mut out, size);
        put_u16(&mut out, name_len);
        put_u16(&mut out, 0); // no extra field
        out.extend_from_slice(name);
        out.extend_from_slice(data);

        put_u32(&mut central, 0x0201_4b50); // central directory header
        put_u16(&mut central, 20); // version made by
        put_u16(&mut central, 20); // version needed
        put_u16(&mut central, 0x0800);
        put_u16(&mut central, 0);
        put_u16(&mut central, dos_time);
        put_u16(&mut central, dos_date);
        put_u32(&mut central, crc);
        put_u32(&mut central, size);
        put_u32(&mut central, size);
        put_u16(&mut central, name_len);
        put_u16(&mut central, 0); // extra
        put_u16(&mut central, 0); // comment
        put_u16(&mut central, 0); // disk
        put_u16(&mut central, 0); // internal attributes
        put_u32(&mut central, 0); // external attributes
        put_u32(&mut central, offset);
        central.extend_from_slice(name);
    }

    let central_offset = out.len() as u32;
    let central_size = central.len() as u32;
    out.extend_from_slice(&central);
    let count = u16::try_from(files.len()).expect("archive entry count is small");
    put_u32(&mut out, 0x0605_4b50); // end of central directory
    put_u16(&mut out, 0);
    put_u16(&mut out, 0);
    put_u16(&mut out, count);
    put_u16(&mut out, count);
    put_u32(&mut out, central_size);
    put_u32(&mut out, central_offset);
    put_u16(&mut out, 0); // no comment
    out
}

fn put_u16(out: &mut Vec<u8>, value: u16) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn put_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}

/// MS-DOS date and time, the only timestamp format a zip entry has.
fn dos_datetime(at: DateTime<Utc>) -> (u16, u16) {
    let year = at.year().clamp(1980, 2107) as u16;
    let date = ((year - 1980) << 9) | ((at.month() as u16) << 5) | at.day() as u16;
    let time = ((at.hour() as u16) << 11) | ((at.minute() as u16) << 5) | (at.second() as u16 / 2);
    (time, date)
}

/// CRC-32 (IEEE 802.3), the checksum zip entries carry.
fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &byte in data {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

#[cfg(test)]
mod tests {
    use super::*;
    use stayproductive_core::State;

    fn at(text: &str) -> Timestamp {
        text.parse().expect("timestamp")
    }

    #[test]
    fn a_fresh_state_exports_a_manifest_without_usage_files() {
        let state = State::fixture();
        let files = entries(&state, at("2026-11-04T10:00:00Z"), true);
        let names: Vec<&str> = files.iter().map(|(name, _)| name.as_str()).collect();
        assert_eq!(
            names,
            [
                "manifest.json",
                "profiles.json",
                "settings.json",
                "commitments.json"
            ]
        );
        let manifest: Value = serde_json::from_slice(&files[0].1).expect("manifest json");
        assert_eq!(manifest["format"], EXPORT_FORMAT);
        assert_eq!(manifest["version"], EXPORT_VERSION);
        assert_eq!(manifest["usage_included"], true);
    }

    #[test]
    fn history_is_optional_and_grouped_by_month() {
        let mut state = State::fixture();
        state.usage.foreground(
            stayproductive_core::AppKey::new("firefox").expect("app"),
            at("2026-10-31T23:59:00Z"),
        );
        state.usage.close_foreground(at("2026-11-01T00:01:00Z"));

        let with_usage = entries(&state, at("2026-11-04T10:00:00Z"), true);
        let october = with_usage
            .iter()
            .find(|(name, _)| name == "usage/2026-10.json")
            .expect("october usage file");
        let body: Value = serde_json::from_slice(&october.1).expect("usage json");
        assert_eq!(body["intervals"][0]["app"], "firefox");

        let without_usage = entries(&state, at("2026-11-04T10:00:00Z"), false);
        assert!(
            without_usage
                .iter()
                .all(|(name, _)| !name.starts_with("usage/"))
        );
    }

    #[test]
    fn the_archive_is_a_readable_stored_zip() {
        let files = vec![("a.txt".to_owned(), b"hello".to_vec())];
        let bytes = archive(&files, at("2026-11-04T10:00:00Z"));
        assert_eq!(&bytes[0..4], b"PK\x03\x04");
        assert_eq!(&bytes[bytes.len() - 22..bytes.len() - 18], b"PK\x05\x06");
        let central_at = u32::from_le_bytes(
            bytes[bytes.len() - 6..bytes.len() - 2]
                .try_into()
                .expect("offset"),
        ) as usize;
        assert_eq!(&bytes[central_at..central_at + 4], b"PK\x01\x02");
        let stored_crc = u32::from_le_bytes(
            bytes[central_at + 16..central_at + 20]
                .try_into()
                .expect("crc"),
        );
        assert_eq!(stored_crc, crc32(b"hello"));
    }

    fn commitment(now: Timestamp, hours: i64) -> stayproductive_core::Commitment {
        stayproductive_core::Commitment::new(
            stayproductive_core::CommitmentKind::Strict,
            now,
            Some(now + chrono::TimeDelta::hours(hours)),
            stayproductive_core::DeactivationMethod::Cooldown(std::time::Duration::from_secs(600)),
        )
    }

    #[test]
    fn an_unknown_export_version_is_refused() {
        let mut files = BTreeMap::new();
        files.insert(
            "manifest.json".to_owned(),
            br#"{"format":"stayproductive.export","version":99}"#.to_vec(),
        );
        let mut state = State::fixture();
        let error = import(&mut state, &files, at("2026-11-04T10:00:00Z")).expect_err("version");
        assert!(error.contains("not supported"), "{error}");
    }

    #[test]
    fn a_clean_state_is_restored_from_an_export() {
        let now = at("2026-11-04T10:00:00Z");
        let mut source = State::fixture();
        let mut profile = stayproductive_core::Profile::new("Focus", now).expect("profile");
        profile.rules.apps.insert(
            AppKey::new("firefox").expect("app"),
            stayproductive_core::AppRuleEntry {
                id: stayproductive_core::ids::new_id(),
                rule: stayproductive_core::AppRule::Block,
            },
        );
        source.add_profile(profile).expect("add profile");
        source.settings.start_of_day = NaiveTime::from_hms_opt(4, 0, 0).expect("time");
        source.settings.timezone = "Europe/Berlin".parse().expect("zone");
        source
            .add_commitment(commitment(now, 8))
            .expect("commitment");

        let files = entries(&source, now, false);
        let packed = archive(&files, now);
        let read = read_archive(&packed).expect("read archive");
        let mut target = State::fixture();
        let summary = import(&mut target, &read, now).expect("import");

        assert!(summary.replaced);
        assert_eq!(summary.profiles_added, 1);
        assert_eq!(summary.commitments_added, 1);
        assert!(summary.settings_applied);
        assert_eq!(target.profiles[0].name, "Focus");
        assert_eq!(target.settings.start_of_day.to_string(), "04:00:00");
        assert_eq!(target.settings.timezone.name(), "Europe/Berlin");
        assert_eq!(target.commitments.len(), 1);
    }

    #[test]
    fn merging_never_weakens_an_active_commitment() {
        let now = at("2026-11-04T10:00:00Z");
        let mut source = State::fixture();
        source
            .add_profile(stayproductive_core::Profile::new("Imported", now).expect("profile"))
            .expect("add profile");
        source
            .add_commitment(commitment(now, 2))
            .expect("commitment");

        let mut target = State::fixture();
        target
            .add_profile(stayproductive_core::Profile::new("Existing", now).expect("profile"))
            .expect("add profile");
        target
            .add_commitment(commitment(now, 8))
            .expect("commitment");
        let before = target.commitments.clone();

        let files = entries(&source, now, false);
        let read = read_archive(&archive(&files, now)).expect("read archive");
        let summary = import(&mut target, &read, now).expect("import");

        assert!(!summary.replaced);
        assert_eq!(summary.profiles_added, 1);
        assert_eq!(summary.commitments_skipped, 1);
        assert_eq!(target.commitments, before, "the longer commitment wins");
        assert_eq!(
            target.settings,
            State::fixture().settings,
            "an import never resets the clock settings of a used install"
        );
    }
}
