// SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
//! Time helpers. The LLM writes local times ("YYYY-MM-DD HH:MM:SS") in the user's timezone; storage is UTC unix ms.

use jiff::{civil::DateTime, tz::TimeZone, Timestamp};

/// MIRIX timezone strings look like `Asia/Shanghai (UTC+08:00)`; keep only the region name.
pub fn tz_region(tz: &str) -> &str {
    tz.split(" (").next().unwrap_or(tz).trim()
}

fn zone(tz: &str) -> Result<TimeZone, String> {
    TimeZone::get(tz_region(tz)).map_err(|e| format!("unknown timezone {tz:?}: {e}"))
}

/// Parse a local time string in `tz` to UTC milliseconds. Accepts `YYYY-MM-DD HH:MM:SS`, with optional fractional
/// seconds, and ISO-8601 with an offset or `Z` (which then ignores `tz`).
pub fn parse_local_to_utc_ms(s: &str, tz: &str) -> Result<i64, String> {
    let s = s.trim();
    if s.ends_with('Z') || s.rfind(['+']).is_some_and(|i| i > 10) || s.rfind('-').is_some_and(|i| i > 10 && s.contains('T')) {
        if let Ok(ts) = s.replace(' ', "T").parse::<Timestamp>() {
            return Ok(ts.as_millisecond());
        }
    }
    let dt: DateTime = if s.contains('.') {
        DateTime::strptime("%Y-%m-%d %H:%M:%S%.f", s)
    } else if s.contains('T') {
        DateTime::strptime("%Y-%m-%dT%H:%M:%S", s)
    } else {
        DateTime::strptime("%Y-%m-%d %H:%M:%S", s)
    }
    .map_err(|e| format!("invalid time {s:?} (expected YYYY-MM-DD HH:MM:SS): {e}"))?;
    let z = dt.to_zoned(zone(tz)?).map_err(|e| format!("cannot place {s:?} in {tz:?}: {e}"))?;
    Ok(z.timestamp().as_millisecond())
}

/// UTC ms -> `YYYY-MM-DD HH:MM:SS` in `tz`.
pub fn format_ms_local(ms: i64, tz: &str) -> String {
    match Timestamp::from_millisecond(ms).ok().zip(zone(tz).ok()) {
        Some((ts, z)) => ts.to_zoned(z).strftime("%Y-%m-%d %H:%M:%S").to_string(),
        None => format!("{ms}"),
    }
}

/// Does the local time `hhmm` (`HH:MM`, 24-hour, in `tz`) come round at least once after `from_ms` and no later than `to_ms`? Used to run
/// postponed work "once a day at 03:30": work queued at `from_ms` is due the first time that clock time has passed.
pub fn occurs_between(tz: &str, hhmm: &str, from_ms: i64, to_ms: i64) -> bool {
    let Some((h, m)) = hhmm.split_once(':').and_then(|(h, m)| Some((h.parse::<i8>().ok()?, m.parse::<i8>().ok()?))) else { return false };
    if !(0..24).contains(&h) || !(0..60).contains(&m) {
        return false;
    }
    let (Ok(z), Ok(from)) = (zone(tz), Timestamp::from_millisecond(from_ms)) else { return false };
    let mut date = from.to_zoned(z.clone()).date();
    for _ in 0..3 {
        let Ok(at) = date.at(h, m, 0, 0).to_zoned(z.clone()) else { return false };
        let t = at.timestamp().as_millisecond();
        if t > from_ms {
            return t <= to_ms;
        }
        match date.tomorrow() {
            Ok(d) => date = d,
            Err(_) => return false,
        }
    }
    false
}

/// Current time in `tz`, as shown to the model (`2026-10-10 09:15:02 PM WEST+0100`).
pub fn now_local_string(tz: &str) -> String {
    match zone(tz) {
        Ok(z) => Timestamp::now().to_zoned(z).strftime("%Y-%m-%d %I:%M:%S %p %Z%z").to_string(),
        Err(_) => Timestamp::now().to_string(),
    }
}

/// Current time as `YYYY-MM-DD HH:MM:SS` (24-hour) in `tz`; used in the system prompt like MIRIX.
pub fn now_local_24h(tz: &str) -> String {
    match zone(tz) {
        Ok(z) => Timestamp::now().to_zoned(z).strftime("%Y-%m-%d %H:%M:%S").to_string(),
        Err(_) => Timestamp::now().strftime("%Y-%m-%d %H:%M:%S").to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_time_of_day_comes_round_after_the_work_was_queued() {
        // Lisbon in winter is UTC+0: 2026-01-10 14:00 local
        let queued = parse_local_to_utc_ms("2026-01-10 14:00:00", "Europe/Lisbon").unwrap();
        let at = |s: &str| parse_local_to_utc_ms(s, "Europe/Lisbon").unwrap();
        assert!(!occurs_between("Europe/Lisbon", "03:30", queued, at("2026-01-11 03:29:59")), "not yet");
        assert!(occurs_between("Europe/Lisbon", "03:30", queued, at("2026-01-11 03:30:00")));
        assert!(occurs_between("Europe/Lisbon", "03:30", queued, at("2026-01-15 09:00:00")), "overdue still counts");
        assert!(occurs_between("Europe/Lisbon", "18:00", queued, at("2026-01-10 18:00:01")), "later the same day");
        assert!(!occurs_between("Europe/Lisbon", "13:00", queued, at("2026-01-10 23:59:59")), "13:00 had already passed when it was queued");
        assert!(!occurs_between("Europe/Lisbon", "25:00", queued, at("2026-02-10 00:00:00")), "not a time");
    }

    #[test]
    fn local_time_converts_to_utc_with_dst() {
        // Lisbon is UTC+1 in summer (WEST), UTC+0 in winter
        let summer = parse_local_to_utc_ms("2026-07-01 12:00:00", "Europe/Lisbon").unwrap();
        let winter = parse_local_to_utc_ms("2026-01-01 12:00:00", "Europe/Lisbon").unwrap();
        assert_eq!(format_ms_local(summer, "UTC"), "2026-07-01 11:00:00");
        assert_eq!(format_ms_local(winter, "UTC"), "2026-01-01 12:00:00");
        assert_eq!(format_ms_local(summer, "Europe/Lisbon"), "2026-07-01 12:00:00");
    }

    #[test]
    fn mirix_style_timezone_labels_and_fractions_and_iso() {
        let a = parse_local_to_utc_ms("2026-03-05 10:00:00", "Asia/Shanghai (UTC+08:00)").unwrap();
        assert_eq!(format_ms_local(a, "UTC"), "2026-03-05 02:00:00");
        let f = parse_local_to_utc_ms("2026-03-05 10:00:00.250", "UTC").unwrap();
        assert_eq!(f % 1000, 250);
        let z = parse_local_to_utc_ms("2026-03-05T10:00:00Z", "Asia/Tokyo").unwrap();
        assert_eq!(format_ms_local(z, "UTC"), "2026-03-05 10:00:00", "explicit Z ignores the user's timezone");
        let off = parse_local_to_utc_ms("2026-03-05T10:00:00+02:00", "UTC").unwrap();
        assert_eq!(format_ms_local(off, "UTC"), "2026-03-05 08:00:00");
    }

    #[test]
    fn bad_input_is_a_clear_error() {
        assert!(parse_local_to_utc_ms("yesterday", "UTC").unwrap_err().contains("expected YYYY-MM-DD"));
        assert!(parse_local_to_utc_ms("2026-03-05 10:00:00", "Mars/Olympus").unwrap_err().contains("unknown timezone"));
        assert!(now_local_string("UTC").contains("UTC"));
    }
}
