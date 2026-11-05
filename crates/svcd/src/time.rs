//! Wall-clock anchor for trace timestamps. The executor itself only ever
//! reads the virtual monotonic clock; wall time is `anchor + monotonic`.

/// Parse `YYYY-MM-DDThh:mm:ss[.fff]Z` into microseconds since the Unix epoch.
pub fn parse_iso_utc_us(s: &str) -> Option<u64> {
    let s = s.strip_suffix('Z')?;
    let (date, time) = s.split_once('T')?;
    let mut d = date.split('-');
    let (y, m, day): (i64, u32, u32) = (d.next()?.parse().ok()?, d.next()?.parse().ok()?, d.next()?.parse().ok()?);
    if d.next().is_some() || !(1..=12).contains(&m) || !(1..=31).contains(&day) {
        return None;
    }
    let (hms, frac) = match time.split_once('.') {
        Some((a, b)) => (a, Some(b)),
        None => (time, None),
    };
    let mut t = hms.split(':');
    let (hh, mm, ss): (u64, u64, u64) = (t.next()?.parse().ok()?, t.next()?.parse().ok()?, t.next()?.parse().ok()?);
    if t.next().is_some() || hh > 23 || mm > 59 || ss > 60 {
        return None;
    }
    let micros = match frac {
        Some(f) if !f.is_empty() && f.len() <= 6 && f.bytes().all(|b| b.is_ascii_digit()) => {
            f.parse::<u64>().ok()? * 10u64.pow(6 - f.len() as u32)
        }
        Some(_) => return None,
        None => 0,
    };
    let days = days_from_civil(y, m, day);
    if days < 0 {
        return None;
    }
    Some(((days as u64 * 86_400 + hh * 3600 + mm * 60 + ss) * 1_000_000) + micros)
}

/// Howard Hinnant's days-from-civil (public domain algorithm).
fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m as i64 + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_known_instants() {
        assert_eq!(parse_iso_utc_us("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse_iso_utc_us("2026-10-24T18:00:00Z"), Some(1_792_864_800_000_000));
        assert_eq!(parse_iso_utc_us("2000-03-01T00:00:00.5Z"), Some(951_868_800_500_000));
        assert_eq!(parse_iso_utc_us("2026-10-24 18:00:00Z"), None);
        assert_eq!(parse_iso_utc_us("2026-13-01T00:00:00Z"), None);
        assert_eq!(parse_iso_utc_us("2026-10-24T18:00:00+02:00"), None);
    }
}
