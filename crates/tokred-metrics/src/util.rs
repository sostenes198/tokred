//! Small numeric and calendar helpers shared by the metrics modules.

/// Seconds in a day.
pub(crate) const SECS_PER_DAY: i64 = 86_400;

/// Token and byte counters are far below 2^53, so the conversion to `f64` is exact in practice.
#[allow(clippy::cast_precision_loss)]
pub(crate) fn to_f64(value: u64) -> f64 {
    value as f64
}

/// `usize` length as `u64` (lossless on every supported platform).
pub(crate) fn len_u64(len: usize) -> u64 {
    u64::try_from(len).unwrap_or(u64::MAX)
}

/// Rounds to 6 decimals (micro-dollars, microseconds) so JSON output is stable and free of
/// floating-point noise such as `0.30000000000000004`.
pub(crate) fn round6(value: f64) -> f64 {
    (value * 1_000_000.0).round() / 1_000_000.0
}

/// `numerator / denominator`, or `None` when the denominator is zero (never NaN or infinity).
pub(crate) fn ratio(numerator: u64, denominator: u64) -> Option<f64> {
    (denominator != 0).then(|| round6(to_f64(numerator) / to_f64(denominator)))
}

/// UTC calendar day (`YYYY-MM-DD`) of a unix timestamp, using the civil-from-days algorithm so no
/// fallible date constructor is needed.
pub(crate) fn day_string(ts: i64) -> String {
    let z = ts.div_euclid(SECS_PER_DAY) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn days_are_utc_calendar_dates() {
        assert_eq!(day_string(0), "1970-01-01");
        assert_eq!(day_string(86_399), "1970-01-01");
        assert_eq!(day_string(86_400), "1970-01-02");
        assert_eq!(day_string(-1), "1969-12-31");
        assert_eq!(day_string(951_782_400), "2000-02-29");
        assert_eq!(day_string(1_751_328_000), "2025-07-01");
        assert_eq!(day_string(1_767_225_599), "2025-12-31");
        assert_eq!(day_string(1_767_225_600), "2026-01-01");
    }

    #[test]
    fn ratios_never_divide_by_zero() {
        assert_eq!(ratio(1, 0), None);
        assert_eq!(ratio(0, 0), None);
        assert_eq!(ratio(1, 4), Some(0.25));
        assert_eq!(ratio(1, 3), Some(0.333_333));
    }

    #[test]
    fn rounding_removes_float_noise() {
        assert!((round6(0.1 + 0.2) - 0.3).abs() < f64::EPSILON);
        assert_eq!(len_u64(3), 3);
        assert!((to_f64(5) - 5.0).abs() < f64::EPSILON);
    }
}
