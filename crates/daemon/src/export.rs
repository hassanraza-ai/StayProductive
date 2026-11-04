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

use chrono::{DateTime, Datelike, Timelike, Utc};
use serde_json::{Value, json};
use stayproductive_core::{AppKey, State, Target, Timestamp};

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
}
