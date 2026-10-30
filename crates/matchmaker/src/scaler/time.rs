//! Clock and timestamp helpers for the Lease.
//!
//! A Lease's `renewTime`/`acquireTime` are MicroTimes: RFC 3339 with exactly six fractional
//! digits. `2024-05-06T07:08:09Z` is rejected by the API server, so [`micro_time`] always writes
//! them. Nothing here needs a date crate; both directions are a few lines of arithmetic.

use std::time::{SystemTime, UNIX_EPOCH};

pub(super) fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs() as i64)
        .unwrap_or_default()
}

/// `2024-05-06T07:08:09.000000Z`, the form the API server accepts for a MicroTime.
pub(super) fn micro_time(secs: i64) -> String {
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.000000Z",
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    )
}

/// Parse an RFC 3339 timestamp, which is what a Lease's `renewTime` holds. Fractional seconds
/// are ignored (the API server writes microseconds) and a numeric offset is honoured.
pub(super) fn parse_rfc3339(text: &str) -> Option<i64> {
    let text = text.trim();
    let number = |range: std::ops::Range<usize>| text.get(range)?.parse::<i64>().ok();
    let (year, month, day) = (number(0..4)?, number(5..7)?, number(8..10)?);
    let (hour, minute, second) = (number(11..13)?, number(14..16)?, number(17..19)?);
    if text.get(4..5)? != "-" || text.get(7..8)? != "-" || text.get(10..11)? != "T" {
        return None;
    }
    if text.get(13..14)? != ":" || text.get(16..17)? != ":" {
        return None;
    }
    let mut rest = text.get(19..)?;
    if let Some(fraction) = rest.strip_prefix('.') {
        let digits = fraction.chars().take_while(char::is_ascii_digit).count();
        if digits == 0 {
            return None;
        }
        rest = fraction.get(digits..)?;
    }
    let offset = match rest {
        "Z" | "z" => 0,
        _ => {
            let sign = match rest.get(0..1)? {
                "+" => 1,
                "-" => -1,
                _ => return None,
            };
            if rest.get(3..4)? != ":" {
                return None;
            }
            let hours = rest.get(1..3)?.parse::<i64>().ok()?;
            let minutes = rest.get(4..6)?.parse::<i64>().ok()?;
            sign * (hours * 3600 + minutes * 60)
        }
    };
    Some(days_from_civil(year, month, day) * 86_400 + hour * 3600 + minute * 60 + second - offset)
}

/// Days since 1970-01-01 for a proleptic Gregorian date (Howard Hinnant's algorithm).
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = year - era * 400;
    let day_of_year = (153 * ((month + 9) % 12) + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let shifted = days + 719_468;
    let era = if shifted >= 0 {
        shifted
    } else {
        shifted - 146_096
    } / 146_097;
    let day_of_era = shifted - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = if month_prime < 10 {
        month_prime + 3
    } else {
        month_prime - 9
    };
    (if month <= 2 { year + 1 } else { year }, month, day)
}
