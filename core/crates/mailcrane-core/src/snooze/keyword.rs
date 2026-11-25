//! The `$snoozed-until-<UTC YYYYMMDDTHHMMZ>` keyword that tells other
//! installs when a message in the Snoozed folder wakes (M4-06).

/// Every snooze keyword starts with this.
pub const KEYWORD_PREFIX: &str = "$snoozed-until-";

/// The keyword for `wake_at` (unix seconds UTC), to the minute (rounded
/// down). `None` for times before 1970 or after 9999.
pub fn format_keyword(wake_at: i64) -> Option<String> {
    if wake_at < 0 {
        return None;
    }
    let days = wake_at.div_euclid(86_400);
    let secs = wake_at.rem_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    if y > 9999 {
        return None;
    }
    Some(format!(
        "{KEYWORD_PREFIX}{y:04}{m:02}{d:02}T{:02}{:02}Z",
        secs / 3600,
        secs % 3600 / 60
    ))
}

/// The wake time in a keyword, or `None` if it isn't a valid one.
pub fn parse_keyword(keyword: &str) -> Option<i64> {
    let rest = keyword
        .get(..KEYWORD_PREFIX.len())
        .filter(|p| p.eq_ignore_ascii_case(KEYWORD_PREFIX))
        .and_then(|_| keyword.get(KEYWORD_PREFIX.len()..))?;
    let b = rest.as_bytes();
    if b.len() != 14
        || !b[8].eq_ignore_ascii_case(&b'T')
        || !b[13].eq_ignore_ascii_case(&b'Z')
        || !b[..8].iter().chain(&b[9..13]).all(u8::is_ascii_digit)
    {
        return None;
    }
    let num = |r: std::ops::Range<usize>| rest.get(r)?.parse::<i64>().ok();
    let (y, m, d) = (num(0..4)?, num(4..6)?, num(6..8)?);
    let (hh, mm) = (num(9..11)?, num(11..13)?);
    if !(1..=12).contains(&m) || d < 1 || d > days_in_month(y, m) || hh > 23 || mm > 59 {
        return None;
    }
    Some(days_from_civil(y, m, d) * 86_400 + hh * 3600 + mm * 60)
}

fn days_in_month(y: i64, m: i64) -> i64 {
    match m {
        2 if (y % 4 == 0 && y % 100 != 0) || y % 400 == 0 => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

// Howard Hinnant's algorithms (public domain), for the proleptic Gregorian
// calendar.
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keywords_round_trip_to_the_minute() {
        // 2026-11-25 08:00:00 UTC.
        let t = 1_795_593_600;
        assert_eq!(
            format_keyword(t).as_deref(),
            Some("$snoozed-until-20261125T0800Z")
        );
        assert_eq!(parse_keyword("$snoozed-until-20261125T0800Z"), Some(t));
        assert_eq!(format_keyword(t + 59), format_keyword(t));
        assert_eq!(
            format_keyword(0).as_deref(),
            Some("$snoozed-until-19700101T0000Z")
        );
        assert_eq!(
            parse_keyword("$Snoozed-Until-20240229t2359z"),
            Some(1_709_251_140)
        );
        for t in [
            0,
            951_782_400,
            1_709_164_800,
            4_102_444_799,
            253_402_300_740,
        ] {
            let k = format_keyword(t).expect("formats");
            assert_eq!(parse_keyword(&k), Some(t - t % 60), "{k}");
        }
        assert_eq!(format_keyword(-1), None);
        assert_eq!(format_keyword(253_402_300_800), None);
    }

    #[test]
    fn bad_keywords_are_ignored() {
        for bad in [
            "",
            "$snoozed-until-",
            "$snoozed-until-20261125T0800",
            "$snoozed-until-20261125X0800Z",
            "$snoozed-until-20261325T0800Z",
            "$snoozed-until-20230229T0800Z",
            "$snoozed-until-20261125T2400Z",
            "$snoozed-until-20261125T0860Z",
            "$snoozed-until-2026112aT0800Z",
            "$snoozed-until-+0261125T0800Z",
            "$snoozed-until-20261125T0800Zx",
            "$other-20261125T0800Z",
        ] {
            assert_eq!(parse_keyword(bad), None, "{bad}");
        }
    }
}
