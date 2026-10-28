//! Pure number formatting helpers: currency, counts, bytes, percentages and durations.
//!
//! Negative or non-finite inputs never produce a minus sign in a place where the
//! quantity cannot be negative: they are clamped to zero, or shown as `n/a` when
//! they are not a number.

/// Placeholder shown for a value that is not a number.
const NOT_A_NUMBER: &str = "n/a";

/// Converts a counter to `f64` for charting and ratios.
///
/// Precision above 2^53 is irrelevant for charts.
#[allow(clippy::cast_precision_loss)]
#[must_use]
pub(crate) fn to_f64(n: u64) -> f64 {
    n as f64
}

/// Inserts `,` thousands separators into a string of ASCII digits.
fn group_thousands(digits: &str) -> String {
    let len = digits.len();
    let mut out = String::with_capacity(len + len / 3);
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (len - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

/// Formats an unsigned integer with thousands separators (`1234567` -> `1,234,567`).
#[must_use]
pub fn integer(n: u64) -> String {
    group_thousands(&n.to_string())
}

/// Formats a non-negative amount rounded to a whole number with thousands separators.
///
/// Negative values clamp to zero and non-finite values give `n/a`.
#[must_use]
pub fn integer_f64(value: f64) -> String {
    if !value.is_finite() {
        return NOT_A_NUMBER.to_owned();
    }
    group_thousands(&format!("{:.0}", value.max(0.0)))
}

/// Formats a USD amount: two decimals, four below one cent, thousands separators.
///
/// `0.0012` -> `$0.0012`, `1234.5` -> `$1,234.50`, `-3.0` -> `-$3.00`. A value that
/// rounds to zero never gets a minus sign.
#[must_use]
pub fn usd(value: f64) -> String {
    if !value.is_finite() {
        return NOT_A_NUMBER.to_owned();
    }
    let abs = value.abs();
    let decimals = if abs > 0.0 && abs < 0.01 { 4 } else { 2 };
    let text = format!("{abs:.decimals$}");
    let (whole, fraction) = text.split_once('.').unwrap_or((&text, ""));
    let negative = value < 0.0 && text.bytes().any(|b| (b'1'..=b'9').contains(&b));
    let sign = if negative { "-" } else { "" };
    format!("{sign}${}.{fraction}", group_thousands(whole))
}

/// Scales `n` to the largest unit in `tiers` (ascending) with one decimal.
///
/// A value that rounds up to the next unit is promoted (`1023.99 KiB` shows as
/// `1.0 MiB`).
fn scale(n: u64, tiers: &[(u128, &str)]) -> String {
    let n = u128::from(n);
    let tenths = |tier: u128| (n * 10 + tier / 2) / tier;
    let mut index = tiers.iter().rposition(|(tier, _)| n >= *tier).unwrap_or(0);
    let promote_at = tiers
        .get(index + 1)
        .map(|(next, _)| 10 * next / tiers[index].0);
    if promote_at.is_some_and(|limit| tenths(tiers[index].0) >= limit) {
        index += 1;
    }
    let (tier, suffix) = tiers[index];
    let value = tenths(tier);
    format!("{}.{}{suffix}", value / 10, value % 10)
}

/// Formats a count compactly (`999` -> `999`, `12_345` -> `12.3k`, `2_000_000` -> `2.0M`).
#[must_use]
pub fn compact(n: u64) -> String {
    const TIERS: [(u128, &str); 3] = [(1_000, "k"), (1_000_000, "M"), (1_000_000_000, "B")];
    if n < 1_000 {
        return n.to_string();
    }
    scale(n, &TIERS)
}

/// Formats a size in bytes with binary units (`512 B`, `1.5 KiB`, `3.0 MiB`).
#[must_use]
pub fn bytes(n: u64) -> String {
    const TIERS: [(u128, &str); 4] = [
        (1 << 10, " KiB"),
        (1 << 20, " MiB"),
        (1 << 30, " GiB"),
        (1 << 40, " TiB"),
    ];
    if n < 1_024 {
        return format!("{n} B");
    }
    scale(n, &TIERS)
}

/// Formats a fraction as a percentage with one decimal (`0.425` -> `42.5%`).
///
/// Negative fractions clamp to `0.0%`.
#[must_use]
pub fn percent(fraction: f64) -> String {
    if !fraction.is_finite() {
        return NOT_A_NUMBER.to_owned();
    }
    format!("{:.1}%", fraction.max(0.0) * 100.0)
}

/// Formats a duration given in milliseconds.
///
/// `0.42` -> `0.42 ms`, `12.34` -> `12.3 ms`, `1500.0` -> `1.50 s`, `125_000.0` -> `2 min 5 s`.
/// Negative values clamp to zero.
#[must_use]
pub fn ms(value: f64) -> String {
    if !value.is_finite() {
        return NOT_A_NUMBER.to_owned();
    }
    let value = value.max(0.0);
    if value < 10.0 {
        format!("{value:.2} ms")
    } else if value < 1_000.0 {
        format!("{value:.1} ms")
    } else if value < 60_000.0 {
        format!("{:.2} s", value / 1_000.0)
    } else {
        let minutes = (value / 60_000.0).floor();
        let seconds = ((value - minutes * 60_000.0) / 1_000.0).floor();
        format!("{minutes:.0} min {seconds:.0} s")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integer_groups_thousands() {
        assert_eq!(integer(0), "0");
        assert_eq!(integer(999), "999");
        assert_eq!(integer(1_000), "1,000");
        assert_eq!(integer(1_234_567), "1,234,567");
        assert_eq!(integer(u64::MAX), "18,446,744,073,709,551,615");
    }

    #[test]
    fn integer_f64_rounds_and_clamps() {
        assert_eq!(integer_f64(1234.6), "1,235");
        assert_eq!(integer_f64(-4.0), "0");
        assert_eq!(integer_f64(f64::INFINITY), "n/a");
    }

    #[test]
    fn usd_picks_a_sensible_precision() {
        assert_eq!(usd(0.0), "$0.00");
        assert_eq!(usd(0.0012), "$0.0012");
        assert_eq!(usd(0.01), "$0.01");
        assert_eq!(usd(1.5), "$1.50");
        assert_eq!(usd(1234.5), "$1,234.50");
        assert_eq!(usd(1_234_567.891), "$1,234,567.89");
        assert_eq!(usd(-3.0), "-$3.00");
    }

    #[test]
    fn usd_never_shows_negative_zero() {
        assert_eq!(usd(-0.0), "$0.00");
        assert_eq!(usd(-0.000_000_1), "$0.0000");
    }

    #[test]
    fn usd_handles_non_finite_values() {
        assert_eq!(usd(f64::NAN), "n/a");
        assert_eq!(usd(f64::INFINITY), "n/a");
    }

    #[test]
    fn compact_uses_suffixes() {
        assert_eq!(compact(0), "0");
        assert_eq!(compact(999), "999");
        assert_eq!(compact(1_000), "1.0k");
        assert_eq!(compact(12_345), "12.3k");
        assert_eq!(compact(2_000_000), "2.0M");
        assert_eq!(compact(3_500_000_000), "3.5B");
    }

    #[test]
    fn compact_promotes_values_that_round_up() {
        assert_eq!(compact(999_960), "1.0M");
        assert_eq!(compact(999_900), "999.9k");
    }

    #[test]
    fn compact_keeps_the_top_unit_for_huge_values() {
        assert_eq!(compact(u64::MAX), "18446744073.7B");
    }

    #[test]
    fn bytes_uses_binary_units() {
        assert_eq!(bytes(0), "0 B");
        assert_eq!(bytes(1_023), "1023 B");
        assert_eq!(bytes(1_024), "1.0 KiB");
        assert_eq!(bytes(1_536), "1.5 KiB");
        assert_eq!(bytes(3 << 20), "3.0 MiB");
        assert_eq!(bytes(5 << 30), "5.0 GiB");
        assert_eq!(bytes(2 << 40), "2.0 TiB");
        assert_eq!(bytes(u64::MAX), "16777216.0 TiB");
    }

    #[test]
    fn bytes_promotes_values_that_round_up() {
        assert_eq!(bytes((1 << 20) - 1), "1.0 MiB");
    }

    #[test]
    fn percent_formats_fractions() {
        assert_eq!(percent(0.0), "0.0%");
        assert_eq!(percent(0.425), "42.5%");
        assert_eq!(percent(1.0), "100.0%");
        assert_eq!(percent(-0.2), "0.0%");
        assert_eq!(percent(f64::NAN), "n/a");
    }

    #[test]
    fn ms_scales_its_unit() {
        assert_eq!(ms(0.0), "0.00 ms");
        assert_eq!(ms(0.42), "0.42 ms");
        assert_eq!(ms(12.34), "12.3 ms");
        assert_eq!(ms(1_500.0), "1.50 s");
        assert_eq!(ms(125_000.0), "2 min 5 s");
        assert_eq!(ms(-5.0), "0.00 ms");
        assert_eq!(ms(f64::NAN), "n/a");
    }

    #[test]
    fn to_f64_converts_counters() {
        assert!((to_f64(42) - 42.0).abs() < f64::EPSILON);
    }
}
