use std::time::{SystemTime, UNIX_EPOCH};

/// Formats a wall-clock instant as RFC 3339 UTC with millisecond precision, as the journal
/// contract requires (`2026-10-02T17:04:05.123Z`).
#[must_use]
pub fn rfc3339_millis(at: SystemTime) -> String {
    let (date, seconds, micros) = parts(at);
    format!("{date}T{seconds}.{:03}Z", micros / 1_000)
}

/// Formats a wall-clock instant the way Elixir's `DateTime.to_iso8601/1` renders a
/// `DateTime.utc_now/0` value: six fractional digits and a `Z`.
#[must_use]
pub fn iso8601_micros(at: SystemTime) -> String {
    let (date, seconds, micros) = parts(at);
    format!("{date}T{seconds}.{micros:06}Z")
}

fn parts(at: SystemTime) -> (String, String, u32) {
    // An instant before 1970 is a broken host clock; it is rendered as the epoch rather than
    // failing a response or a journal line over it.
    let since_epoch = at.duration_since(UNIX_EPOCH).unwrap_or_default();
    let total_seconds = since_epoch.as_secs();
    let days = total_seconds / 86_400;
    let of_day = total_seconds % 86_400;
    let (year, month, day) = civil_from_days(days);
    (
        format!("{year:04}-{month:02}-{day:02}"),
        format!(
            "{:02}:{:02}:{:02}",
            of_day / 3_600,
            (of_day % 3_600) / 60,
            of_day % 60
        ),
        since_epoch.subsec_micros(),
    )
}

/// Converts days since 1970-01-01 to a proleptic Gregorian date.
///
/// This is Howard Hinnant's `civil_from_days`, restricted to non-negative day counts, which is
/// every instant this service can observe.
const fn civil_from_days(days: u64) -> (u64, u64, u64) {
    let shifted = days + 719_468;
    let era = shifted / 146_097;
    let day_of_era = shifted - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    };
    let year = year_of_era + era * 400 + if month <= 2 { 1 } else { 0 };
    (year, month, day)
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, UNIX_EPOCH};

    use super::{iso8601_micros, rfc3339_millis};

    #[test]
    fn formats_the_epoch() {
        assert_eq!(rfc3339_millis(UNIX_EPOCH), "1970-01-01T00:00:00.000Z");
    }

    #[test]
    fn formats_a_known_instant_in_both_precisions() {
        // 2026-10-02T17:04:05.123456Z
        let at = UNIX_EPOCH + Duration::from_micros(1_790_960_645_123_456);

        assert_eq!(rfc3339_millis(at), "2026-10-02T17:04:05.123Z");
        assert_eq!(iso8601_micros(at), "2026-10-02T17:04:05.123456Z");
    }

    #[test]
    fn handles_a_leap_day() {
        // 2024-02-29T00:00:01Z
        let at = UNIX_EPOCH + Duration::from_secs(1_709_164_801);

        assert_eq!(rfc3339_millis(at), "2024-02-29T00:00:01.000Z");
    }
}
