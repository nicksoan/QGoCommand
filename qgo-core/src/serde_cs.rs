//! Serde adapters that reproduce the exact on-disk JSON shapes written by
//! `System.Text.Json` in QGo.App, so the Rust port reads and writes the same
//! files as the WPF app without migrating anything.
//!
//! Three C# behaviours have to be matched:
//!
//! 1. `DateTime` is written as an ISO-8601 string. `DateTime.UtcNow` values get
//!    a `Z` suffix, `DateTime.Now` values get a numeric offset (`+00:00`), and
//!    `DateTime.MinValue` is written bare (`0001-01-01T00:00:00`). The fraction
//!    is up to 7 digits (ticks) with trailing zeros trimmed, and omitted when 0.
//! 2. `Storage.Opt` sets `NumberHandling = AllowNamedFloatingPointLiterals`, so
//!    `double.NaN` round-trips as the *string* `"NaN"` (see the `WindowLeft` /
//!    `WindowTop` / `WindowHeight` defaults in `AppSettings`).
//! 3. No `JsonStringEnumConverter` is registered, so `ExecutionMode` is a plain
//!    integer on disk (`0` = Simultaneous, `1` = Sequential).

use chrono::{DateTime, Datelike, Local, NaiveDate, NaiveDateTime, TimeZone, Timelike, Utc};
use serde::de::{self, Visitor};
use serde::{Deserializer, Serializer};
use std::fmt;

/// The Rust equivalent of C#'s `DateTime.MinValue` (`0001-01-01T00:00:00`).
pub fn min_value() -> DateTime<Utc> {
    Utc.from_utc_datetime(
        &NaiveDate::from_ymd_opt(1, 1, 1)
            .expect("0001-01-01 is a valid date")
            .and_hms_opt(0, 0, 0)
            .expect("00:00:00 is a valid time"),
    )
}

/// True when `dt` is `DateTime.MinValue`, which C# writes without any zone marker.
fn is_min_value(dt: &DateTime<Utc>) -> bool {
    dt.year() == 1 && dt.month() == 1 && dt.day() == 1 && dt.num_seconds_from_midnight() == 0
}

/// Renders the fractional part the way `System.Text.Json` does: up to 7 digits
/// (one tick = 100ns), trailing zeros trimmed, omitted entirely when zero.
fn ticks_fraction(nanos: u32) -> String {
    let ticks = nanos / 100; // 0..=9_999_999
    if ticks == 0 {
        return String::new();
    }
    let mut s = format!("{ticks:07}");
    while s.ends_with('0') {
        s.pop();
    }
    format!(".{s}")
}

/// Formats as C# formats a `DateTime` with `Kind == Utc` (trailing `Z`).
pub fn format_utc(dt: &DateTime<Utc>) -> String {
    if is_min_value(dt) {
        return "0001-01-01T00:00:00".to_string();
    }
    format!(
        "{}{}Z",
        dt.format("%Y-%m-%dT%H:%M:%S"),
        ticks_fraction(dt.timestamp_subsec_nanos())
    )
}

/// Formats as C# formats a `DateTime` with `Kind == Local` (numeric offset).
///
/// `Shortcut.LastUsed` is assigned from `DateTime.Now`, so it lands on disk in
/// this shape. Values are held in UTC in memory and converted on the way out.
pub fn format_local(dt: &DateTime<Utc>) -> String {
    if is_min_value(dt) {
        return "0001-01-01T00:00:00".to_string();
    }
    let local = dt.with_timezone(&Local);
    format!(
        "{}{}{}",
        local.format("%Y-%m-%dT%H:%M:%S"),
        ticks_fraction(local.timestamp_subsec_nanos()),
        local.format("%:z")
    )
}

/// Parses every shape `System.Text.Json` can emit for a `DateTime`:
/// with offset, with `Z`, or bare (which C# treats as unspecified/local — we
/// read it as UTC, matching how `DateTime.MinValue` and midnight dates behave).
pub fn parse_datetime(s: &str) -> Result<DateTime<Utc>, String> {
    let s = s.trim();
    if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
        return Ok(dt.with_timezone(&Utc));
    }
    for fmt in [
        "%Y-%m-%dT%H:%M:%S%.f",
        "%Y-%m-%dT%H:%M:%S",
        "%Y-%m-%d %H:%M:%S%.f",
        "%Y-%m-%d %H:%M:%S",
    ] {
        if let Ok(naive) = NaiveDateTime::parse_from_str(s, fmt) {
            return Ok(Utc.from_utc_datetime(&naive));
        }
    }
    if let Ok(date) = NaiveDate::parse_from_str(s, "%Y-%m-%d") {
        if let Some(naive) = date.and_hms_opt(0, 0, 0) {
            return Ok(Utc.from_utc_datetime(&naive));
        }
    }
    Err(format!("unrecognised DateTime value: {s:?}"))
}

struct DateTimeVisitor;

impl<'de> Visitor<'de> for DateTimeVisitor {
    type Value = DateTime<Utc>;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("an ISO-8601 date/time string as written by System.Text.Json")
    }

    fn visit_str<E: de::Error>(self, v: &str) -> Result<Self::Value, E> {
        parse_datetime(v).map_err(de::Error::custom)
    }
}

/// `#[serde(with = "serde_cs::datetime_utc")]` — reads any shape, writes `...Z`.
pub mod datetime_utc {
    use super::*;

    pub fn serialize<S: Serializer>(dt: &DateTime<Utc>, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&format_utc(dt))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<DateTime<Utc>, D::Error> {
        d.deserialize_str(DateTimeVisitor)
    }
}

/// `#[serde(with = "serde_cs::datetime_local")]` — reads any shape, writes an offset.
pub mod datetime_local {
    use super::*;

    pub fn serialize<S: Serializer>(dt: &DateTime<Utc>, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&format_local(dt))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<DateTime<Utc>, D::Error> {
        d.deserialize_str(DateTimeVisitor)
    }
}

/// `#[serde(with = "serde_cs::datetime_utc_opt")]` for C# `DateTime?` fields.
pub mod datetime_utc_opt {
    use super::*;

    struct OptVisitor;

    impl<'de> Visitor<'de> for OptVisitor {
        type Value = Option<DateTime<Utc>>;

        fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("an ISO-8601 date/time string or null")
        }

        fn visit_none<E: de::Error>(self) -> Result<Self::Value, E> {
            Ok(None)
        }

        fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
            Ok(None)
        }

        fn visit_some<D: Deserializer<'de>>(self, d: D) -> Result<Self::Value, D::Error> {
            d.deserialize_str(DateTimeVisitor).map(Some)
        }
    }

    pub fn serialize<S: Serializer>(dt: &Option<DateTime<Utc>>, s: S) -> Result<S::Ok, S::Error> {
        match dt {
            Some(dt) => s.serialize_str(&format_utc(dt)),
            None => s.serialize_none(),
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<DateTime<Utc>>, D::Error> {
        d.deserialize_option(OptVisitor)
    }
}

/// `#[serde(with = "serde_cs::cs_f64")]` — the `AllowNamedFloatingPointLiterals`
/// contract: `NaN` / `Infinity` / `-Infinity` travel as JSON *strings*.
pub mod cs_f64 {
    use super::*;

    struct F64Visitor;

    impl<'de> Visitor<'de> for F64Visitor {
        type Value = f64;

        fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("a number, or one of \"NaN\", \"Infinity\", \"-Infinity\"")
        }

        fn visit_f64<E: de::Error>(self, v: f64) -> Result<f64, E> {
            Ok(v)
        }

        fn visit_i64<E: de::Error>(self, v: i64) -> Result<f64, E> {
            Ok(v as f64)
        }

        fn visit_u64<E: de::Error>(self, v: u64) -> Result<f64, E> {
            Ok(v as f64)
        }

        fn visit_unit<E: de::Error>(self) -> Result<f64, E> {
            Ok(f64::NAN)
        }

        fn visit_none<E: de::Error>(self) -> Result<f64, E> {
            Ok(f64::NAN)
        }

        fn visit_some<D: Deserializer<'de>>(self, d: D) -> Result<f64, D::Error> {
            d.deserialize_any(F64Visitor)
        }

        fn visit_str<E: de::Error>(self, v: &str) -> Result<f64, E> {
            match v.trim() {
                "NaN" => Ok(f64::NAN),
                "Infinity" => Ok(f64::INFINITY),
                "-Infinity" => Ok(f64::NEG_INFINITY),
                other => other.parse::<f64>().map_err(de::Error::custom),
            }
        }
    }

    pub fn serialize<S: Serializer>(v: &f64, s: S) -> Result<S::Ok, S::Error> {
        if v.is_nan() {
            s.serialize_str("NaN")
        } else if v.is_infinite() {
            s.serialize_str(if *v > 0.0 { "Infinity" } else { "-Infinity" })
        } else {
            s.serialize_f64(*v)
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<f64, D::Error> {
        d.deserialize_any(F64Visitor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn min_value_is_written_without_a_zone_marker() {
        assert_eq!(format_utc(&min_value()), "0001-01-01T00:00:00");
        assert_eq!(format_local(&min_value()), "0001-01-01T00:00:00");
    }

    #[test]
    fn utc_round_trips_the_shape_written_by_the_wpf_app() {
        // Straight out of the user's usage-stats.json.
        let parsed = parse_datetime("2026-08-05T06:45:35.0177265Z").unwrap();
        assert_eq!(format_utc(&parsed), "2026-08-05T06:45:35.0177265Z");
    }

    #[test]
    fn offset_form_and_bare_form_both_parse() {
        // links.json writes DateTime.Now, hence the offset.
        let with_offset = parse_datetime("2026-01-15T15:50:37.4290492+00:00").unwrap();
        assert_eq!(with_offset.year(), 2026);
        // DateTime.MinValue is written bare.
        let bare = parse_datetime("0001-01-01T00:00:00").unwrap();
        assert_eq!(bare, min_value());
    }

    #[test]
    fn whole_second_timestamps_carry_no_fraction() {
        let parsed = parse_datetime("2026-07-07T00:00:00Z").unwrap();
        assert_eq!(format_utc(&parsed), "2026-07-07T00:00:00Z");
    }

    #[test]
    fn nan_survives_a_json_round_trip_as_a_string() {
        #[derive(serde::Serialize, serde::Deserialize)]
        struct Probe {
            #[serde(with = "cs_f64")]
            v: f64,
        }
        let json = serde_json::to_string(&Probe { v: f64::NAN }).unwrap();
        assert_eq!(json, r#"{"v":"NaN"}"#);
        let back: Probe = serde_json::from_str(&json).unwrap();
        assert!(back.v.is_nan());

        let numeric: Probe = serde_json::from_str(r#"{"v":901.5}"#).unwrap();
        assert_eq!(numeric.v, 901.5);
        // C# writes integral doubles without a decimal point.
        let integral: Probe = serde_json::from_str(r#"{"v":18}"#).unwrap();
        assert_eq!(integral.v, 18.0);
    }
}
