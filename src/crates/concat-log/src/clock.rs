// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! The wall clock as a log writes it, and a length of time as a person
//! reads it.
//!
//! UTC throughout: a log read by somebody else is a log whose times mean
//! the same thing in both places.

use std::fmt::Write as _;
use std::time::Duration;

/// Seconds and milliseconds since the epoch, or zero if the clock is before
/// it - which is not a case worth a branch anywhere else.
pub(crate) fn now() -> (i64, u32) {
    match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
        Ok(since) => (since.as_secs() as i64, since.subsec_millis()),
        Err(_) => (0, 0),
    }
}

/// The day an instant falls on, counted from the epoch: what a line's
/// time of day leaves out, and so what a file says once whenever it
/// changes.
pub(crate) fn day((seconds, _): (i64, u32)) -> i64 {
    seconds.div_euclid(86_400)
}

/// `20260914-123456`, for a file name: sorts as it reads, and has nothing
/// in it a file system anywhere objects to.
pub(crate) fn for_file((seconds, _): (i64, u32)) -> String {
    let (year, month, day) = civil(seconds.div_euclid(86_400));
    let (hour, minute, second) = time_of_day(seconds);
    format!("{year:04}{month:02}{day:02}-{hour:02}{minute:02}{second:02}")
}

/// `2026-09-14`, for the line that says which day the times belong to.
pub(crate) fn date((seconds, _): (i64, u32)) -> String {
    let (year, month, day) = civil(seconds.div_euclid(86_400));
    format!("{year:04}-{month:02}-{day:02}")
}

/// `12:34:56.789` onto the end of `out`: all a line needs once the file
/// has said the day.
pub(crate) fn time_into(out: &mut String, (seconds, millis): (i64, u32)) {
    let (hour, minute, second) = time_of_day(seconds);
    let _ = write!(out, "{hour:02}:{minute:02}:{second:02}.{millis:03}");
}

fn time_of_day(seconds: i64) -> (i64, i64, i64) {
    let rest = seconds.rem_euclid(86_400);
    (rest / 3600, (rest % 3600) / 60, rest % 60)
}

/// The calendar date `days` after 1970-01-01, by Howard Hinnant's
/// `civil_from_days`: the years are shifted to start in March so that the
/// leap day lands at the end of one and the month lengths fall into a
/// pattern with no table in it.
fn civil(days: i64) -> (i64, u32, u32) {
    let shifted = days + 719_468;
    let era = shifted.div_euclid(146_097);
    let day_of_era = shifted.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let march_month = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * march_month + 2) / 5 + 1) as u32;
    let month = if march_month < 10 {
        march_month + 3
    } else {
        march_month - 9
    } as u32;
    (year + i64::from(month <= 2), month, day)
}

/// A length of time in the unit a person would say it in: `840 µs`,
/// `8.4 ms`, `124 ms`, `1.24 s`, `43.2 s`, `2 m 05 s`, `1 h 02 m`. Three figures or
/// so, because a step that took 1.2 s and one that took 1.3 s are the same
/// step, and one that took 12 s is not.
pub fn span(length: Duration) -> String {
    let seconds = length.as_secs_f64();
    if seconds < 0.001 {
        format!("{} µs", length.as_micros())
    } else if seconds < 0.01 {
        format!("{:.1} ms", seconds * 1000.0)
    } else if seconds < 1.0 {
        format!("{:.0} ms", seconds * 1000.0)
    } else if seconds < 10.0 {
        format!("{seconds:.2} s")
    } else if seconds < 60.0 {
        format!("{seconds:.1} s")
    } else if seconds < 3600.0 {
        let whole = seconds.round() as u64;
        format!("{} m {:02} s", whole / 60, whole % 60)
    } else {
        let minutes = (seconds / 60.0).round() as u64;
        format!("{} h {:02} m", minutes / 60, minutes % 60)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_epoch_and_a_leap_day_come_out_right() {
        assert_eq!(civil(0), (1970, 1, 1));
        // 2024-02-29, a leap day, and the day after it.
        assert_eq!(civil(19_782), (2024, 2, 29));
        assert_eq!(civil(19_783), (2024, 3, 1));
        // 2000 is a leap year and 1900 was not; the era arithmetic is what
        // gets that pair wrong when it is wrong at all.
        assert_eq!(civil(11_016), (2000, 2, 29));
        assert_eq!(civil(-25_508), (1900, 3, 1));
    }

    #[test]
    fn an_instant_reads_as_a_file_name_a_date_and_a_time() {
        let at = (1_726_316_096, 789);
        assert_eq!(for_file(at), "20240914-121456");
        assert_eq!(date(at), "2024-09-14");
        let mut time = String::new();
        time_into(&mut time, at);
        assert_eq!(time, "12:14:56.789");
        // A file name sorts the way the runs happened.
        assert!(for_file((0, 0)) < for_file(at));
        assert_eq!(day(at), 19_980);
    }

    #[test]
    fn a_span_is_said_in_the_unit_it_is_about() {
        assert_eq!(span(Duration::from_micros(840)), "840 µs");
        assert_eq!(span(Duration::from_micros(8_400)), "8.4 ms");
        assert_eq!(span(Duration::from_micros(12_400)), "12 ms");
        assert_eq!(span(Duration::from_millis(843)), "843 ms");
        assert_eq!(span(Duration::from_millis(1_240)), "1.24 s");
        assert_eq!(span(Duration::from_millis(43_210)), "43.2 s");
        assert_eq!(span(Duration::from_secs(125)), "2 m 05 s");
        assert_eq!(span(Duration::from_secs(3_725)), "1 h 02 m");
    }
}
