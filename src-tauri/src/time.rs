//! Epoch-millisecond utilities (UTC RFC 3339 formatting) without external
//! crates.
//!
//! Shared by every timestamped McpGo subsystem: the job registry
//! (job_status / job_logs records) and the service store (definition
//! creation/modification times).
//!
//! `rfc3339_utc` implements Howard Hinnant's `civil_from_days` algorithm
//! (http://howardhinnant.github.io/date_algorithms.html) - the same
//! arithmetic used by `std::chrono` date libraries - so the files carry
//! stable, sortable, UTC timestamps with no dependency on a date/time
//! crate.

/// Current UNIX epoch time in milliseconds. Returns 0 if the system clock
/// is set before 1970 (a broken machine, not a normal scenario).
pub fn now_epoch_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

/// Formats epoch milliseconds as `YYYY-MM-DDTHH:MM:SS.mmmZ` (UTC).
pub fn rfc3339_utc(epoch_millis: u64) -> String {
    let days = (epoch_millis / 86_400_000) as i64;
    let millis_of_day = epoch_millis % 86_400_000;
    let (year, month, day) = civil_from_days(days);

    let hour = millis_of_day / 3_600_000;
    let minute = (millis_of_day % 3_600_000) / 60_000;
    let second = (millis_of_day % 60_000) / 1_000;
    let millis = millis_of_day % 1_000;

    format!(
        "{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}.{millis:03}Z"
    )
}

/// Converts a day count relative to 1970-01-01 into a proleptic Gregorian
/// civil date. Valid for any date representable by u64 epoch milliseconds
/// (up to year ~584 million, far beyond any realistic use).
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // [1, 12]
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_the_epoch() {
        assert_eq!(rfc3339_utc(0), "1970-01-01T00:00:00.000Z");
    }

    #[test]
    fn formats_leap_days() {
        // 2000-02-29T00:00:00Z (leap day, divisible-by-400 century).
        assert_eq!(rfc3339_utc(951_782_400_000), "2000-02-29T00:00:00.000Z");
        // 2024-02-29T00:00:00.123Z.
        assert_eq!(rfc3339_utc(1_709_164_800_123), "2024-02-29T00:00:00.123Z");
    }

    #[test]
    fn formats_regular_and_non_leap_century_dates() {
        // 2024-11-10T10:29:27.890Z (verified against an independent
        // implementation of the Gregorian calendar).
        assert_eq!(rfc3339_utc(1_731_234_567_890), "2024-11-10T10:29:27.890Z");
        // 2100-01-01T00:00:00Z (2100 is NOT a leap year: divisible by 100).
        assert_eq!(rfc3339_utc(4_102_444_800_000), "2100-01-01T00:00:00.000Z");
    }

    #[test]
    fn handles_the_last_millisecond_of_a_day() {
        assert_eq!(rfc3339_utc(86_399_999), "1970-01-01T23:59:59.999Z");
        assert_eq!(rfc3339_utc(86_400_000), "1970-01-02T00:00:00.000Z");
    }
}
