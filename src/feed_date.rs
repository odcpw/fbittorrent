//! Dates from untrusted feed and indexer text: RFC 2822 `pubDate` values and
//! RFC 3339 Atom timestamps. RSS refreshes run inside the daemon, where a
//! panic stops the whole app, so every slice here is checked and arithmetic
//! cannot overflow: an unusable date is 0 ("unknown"), never a panic.

const MONTHS: [&str; 12] = ["jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec"];

/// Days since 1970-01-01 for a proleptic Gregorian date (Howard Hinnant).
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

/// "+hhmm"/"-hhmm" as seconds east of UTC; named zones (GMT, UT, Z, EST...)
/// and malformed offsets count as UTC, as before.
fn zone_offset(zone: &str) -> i64 {
    let bytes = zone.as_bytes();
    let sign = match bytes.first() {
        Some(b'+') => 1,
        Some(b'-') => -1,
        _ => return 0,
    };
    let Some(digits) = bytes.get(1..5).filter(|d| d.iter().all(u8::is_ascii_digit)) else { return 0 };
    let value = |pair: &[u8]| i64::from(pair[0] - b'0') * 10 + i64::from(pair[1] - b'0');
    let (hours, minutes) = (value(&digits[..2]), value(&digits[2..]));
    if hours > 23 || minutes > 59 { return 0; }
    sign * (hours * 3600 + minutes * 60)
}

/// Unix seconds for an RFC 2822-ish date ("Fri, 02 Sep 2026 03:58:00 +0000",
/// as qBittorrent and most feeds emit it); 0 when the text is not a date.
pub fn parse_rfc2822(s: &str) -> i64 {
    let s = s.trim();
    let s = s.split_once(", ").map(|(_, rest)| rest).unwrap_or(s); // drop weekday
    let mut it = s.split_whitespace();
    let (Some(d), Some(mon), Some(y), Some(hms)) = (it.next(), it.next(), it.next(), it.next()) else { return 0 };
    let zone = it.next().unwrap_or("+0000");
    let mon = mon.to_ascii_lowercase();
    let Some(m) = MONTHS.iter().position(|name| mon.starts_with(name)) else { return 0 };
    let (Ok(d), Ok(y)) = (d.parse::<i64>(), y.parse::<i64>()) else { return 0 };
    let mut t = hms.split(':').map(|part| part.parse::<i64>().unwrap_or(0));
    let (h, mi, se) = (t.next().unwrap_or(0), t.next().unwrap_or(0), t.next().unwrap_or(0));
    // Bounded fields keep every product below in range; anything else is not a date.
    if !(1..=31).contains(&d) || !(0..=9999).contains(&y) || !(0..=23).contains(&h)
        || !(0..=59).contains(&mi) || !(0..=60).contains(&se) {
        return 0;
    }
    let days = days_from_civil(y, m as i64 + 1, d);
    days * 86400 + h * 3600 + mi * 60 + se - zone_offset(zone)
}

/// RSS `pubDate` or Atom RFC 3339 (`2026-10-09T12:00:00.5+02:00`, `...Z`).
pub fn parse_feed_date(value: &str) -> i64 {
    let value = value.trim();
    if value.len() < 19 || value.as_bytes().get(4) != Some(&b'-') { return parse_rfc2822(value); }
    let parse = |from: usize, to: usize| value.get(from..to).and_then(|v| v.parse::<i64>().ok());
    let (Some(y), Some(m), Some(d), Some(h), Some(mi), Some(s)) =
        (parse(0, 4), parse(5, 7), parse(8, 10), parse(11, 13), parse(14, 16), parse(17, 19)) else { return 0 };
    if !(0..=9999).contains(&y) || !(1..=12).contains(&m) || !(1..=31).contains(&d) || !(0..=23).contains(&h)
        || !(0..=59).contains(&mi) || !(0..=60).contains(&s) {
        return 0;
    }
    // `get(19..)` above succeeded for every field, so 19 is a char boundary.
    let tail = value.get(19..).unwrap_or("");
    let zone = tail.find(['+', '-']).and_then(|index| tail.get(index..)).map(|zone| zone.replace(':', ""));
    days_from_civil(y, m, d) * 86400 + h * 3600 + mi * 60 + s - zone.as_deref().map_or(0, zone_offset)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Independent reference: count days year by year and month by month.
    fn reference(y: i64, m: i64, d: i64, h: i64, mi: i64, s: i64, offset: i64) -> i64 {
        let leap = |y: i64| (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
        let lengths = |y: i64| [31, if leap(y) { 29 } else { 28 }, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
        let mut days = 0i64;
        if y >= 1970 { for year in 1970..y { days += if leap(year) { 366 } else { 365 }; } }
        else { for year in y..1970 { days -= if leap(year) { 366 } else { 365 }; } }
        days += lengths(y)[..(m - 1) as usize].iter().sum::<i64>() + d - 1;
        days * 86400 + h * 3600 + mi * 60 + s - offset
    }

    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 { self.0 ^= self.0 << 13; self.0 ^= self.0 >> 7; self.0 ^= self.0 << 17; self.0 }
        fn below(&mut self, n: u64) -> u64 { self.next() % n }
    }

    #[test]
    fn known_dates_and_zones() {
        let base = 1_791_547_200; // 2026-10-09T12:00:00Z
        assert_eq!(parse_rfc2822("Fri, 09 Oct 2026 12:00:00 +0000"), base);
        assert_eq!(parse_rfc2822("09 Oct 2026 12:00:00 GMT"), base);
        assert_eq!(parse_rfc2822("09 October 2026 14:30:00 +0230"), base);
        assert_eq!(parse_rfc2822("09 Oct 2026 07:00:00 -0500"), base);
        assert_eq!(parse_feed_date("2026-10-09T12:00:00Z"), base);
        assert_eq!(parse_feed_date("2026-10-09T14:00:00.25+02:00"), base);
        assert_eq!(parse_feed_date(" 2026-10-09T08:00:00-04:00 "), base);
        assert_eq!(parse_feed_date("Fri, 09 Oct 2026 12:00:00 +0000"), base);
        // Malformed zones are UTC; malformed dates are unknown.
        assert_eq!(parse_rfc2822("09 Oct 2026 12:00:00 +0é0"), base);
        assert_eq!(parse_rfc2822("09 Oct 2026 12:00:00 +é000"), base);
        assert_eq!(parse_rfc2822("09 Oct 2026 12:00:00 -00é"), base);
        assert_eq!(parse_rfc2822("09 Oct 2026 12:00:00 +9999"), base);
        for bad in ["", "garbage", "32 Oct 2026 12:00:00", "09 Foo 2026 12:00:00", "09 Oct 99999999999999 12:00:00 +0000",
                    "09 Oct 2026 99999999999999999:00:00", "9223372036854775807 Oct 2026 0:0:0", "2026-13-09T12:00:00Z",
                    "2026-10-09T12:00:00é", "２０２６-10-09T12:00:00Z", "2026-1é-09T12:00:00Z"] {
            let _ = parse_feed_date(bad);
            assert!(parse_rfc2822(bad) == 0 || bad.contains('é'), "{bad}");
        }
    }

    #[test]
    fn arbitrary_pub_dates_never_panic_and_valid_ones_match_a_reference() {
        let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
        let alphabet: Vec<char> = "0123456789 :+-,TZ.abcdefghijklmnopqrstuvwxyzJFMASOND\té€😀\u{0}\u{7f}".chars().collect();
        for _ in 0..100_000 {
            let len = rng.below(48) as usize;
            let text: String = (0..len).map(|_| alphabet[rng.below(alphabet.len() as u64) as usize]).collect();
            let _ = parse_feed_date(&text);
            let _ = parse_rfc2822(&text);
        }
        for _ in 0..8_000 {
            let (y, m) = (rng.below(10_000) as i64, rng.below(12) as i64 + 1);
            let d = rng.below(28) as i64 + 1;
            let (h, mi, s) = (rng.below(24) as i64, rng.below(60) as i64, rng.below(60) as i64);
            let (zh, zm) = (rng.below(24) as i64, rng.below(60) as i64);
            let sign = if rng.below(2) == 0 { 1 } else { -1 };
            let zone = format!("{}{zh:02}{zm:02}", if sign > 0 { '+' } else { '-' });
            let expected = reference(y, m, d, h, mi, s, sign * (zh * 3600 + zm * 60));
            let month = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"][(m - 1) as usize];
            let rfc2822 = format!("Mon, {d:02} {month} {y:04} {h:02}:{mi:02}:{s:02} {zone}");
            assert_eq!(parse_rfc2822(&rfc2822), expected, "{rfc2822}");
            let rfc3339 = format!("{y:04}-{m:02}-{d:02}T{h:02}:{mi:02}:{s:02}{}{zh:02}:{zm:02}", if sign > 0 { '+' } else { '-' });
            assert_eq!(parse_feed_date(&rfc3339), expected, "{rfc3339}");
            // Mutations of a valid date: multibyte bytes at every position,
            // truncation at every char boundary, and a damaged zone.
            for (index, _) in rfc2822.char_indices() {
                let mut mutated = rfc2822.clone();
                mutated.insert(index, ['é', '€', '😀'][rng.below(3) as usize]);
                let _ = parse_rfc2822(&mutated);
                let _ = parse_feed_date(&mutated);
                let _ = parse_rfc2822(&rfc2822[..index]);
                let _ = parse_feed_date(&rfc3339[..index.min(rfc3339.len())]);
            }
        }
    }
}
