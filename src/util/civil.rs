//! Calendar dates of Unix times (proleptic Gregorian, UTC).

pub const MS_PER_DAY: u64 = 86_400_000;

/// Days since 1970-01-01 of a date.
pub fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (i64::from(month) + 9) % 12;
    let doy = (153 * mp + 2) / 5 + i64::from(day) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Year and month of a day since 1970-01-01.
pub fn civil_from_days(days: i64) -> (i64, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_civil() {
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(days_from_civil(2026, 11, 3), 20_760);
        assert_eq!(civil_from_days(days_from_civil(2024, 2, 29)), (2024, 2));
        assert_eq!(civil_from_days(days_from_civil(2025, 12, 31)), (2025, 12));
    }
}
