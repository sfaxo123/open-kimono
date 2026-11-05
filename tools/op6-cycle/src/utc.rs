//! Minimal UTC date/time formatting, mirroring the `date -u` formats the
//! bash op6-cycle uses: ISO `%Y-%m-%dT%H:%M:%SZ`, stamp `%Y%m%dT%H%M%SZ`,
//! and `%H:%M:%SZ`. Civil conversion is Howard Hinnant's civil_from_days.

pub struct Civil {
    pub year: i64,
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub min: u32,
    pub sec: u32,
}

pub fn civil_from_epoch(epoch: i64) -> Civil {
    let days = epoch.div_euclid(86_400);
    let secs = epoch.rem_euclid(86_400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097); // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    let year = if m <= 2 { y + 1 } else { y };
    Civil {
        year,
        month: m as u32,
        day: d as u32,
        hour: (secs / 3600) as u32,
        min: ((secs % 3600) / 60) as u32,
        sec: (secs % 60) as u32,
    }
}

/// `date -u '+%Y-%m-%dT%H:%M:%SZ'`
pub fn iso(epoch: i64) -> String {
    let c = civil_from_epoch(epoch);
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        c.year, c.month, c.day, c.hour, c.min, c.sec
    )
}

/// `date -u '+%Y%m%dT%H%M%SZ'`
pub fn stamp(epoch: i64) -> String {
    let c = civil_from_epoch(epoch);
    format!(
        "{:04}{:02}{:02}T{:02}{:02}{:02}Z",
        c.year, c.month, c.day, c.hour, c.min, c.sec
    )
}

/// `date -u '+%H:%M:%SZ'`
pub fn hms(epoch: i64) -> String {
    let c = civil_from_epoch(epoch);
    format!("{:02}:{:02}:{:02}", c.hour, c.min, c.sec)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Vectors cross-checked against GNU `date -u -d @<epoch>` on the 3090.
    #[test]
    fn epoch_zero() {
        assert_eq!(iso(0), "1970-01-01T00:00:00Z");
        assert_eq!(stamp(0), "19700101T000000Z");
        assert_eq!(hms(0), "00:00:00");
    }

    #[test]
    fn known_epochs() {
        assert_eq!(iso(1_767_225_600), "2026-01-01T00:00:00Z");
        assert_eq!(stamp(1_767_225_600), "20260101T000000Z");
        assert_eq!(iso(1_767_229_261), "2026-01-01T01:01:01Z");
        assert_eq!(iso(1_794_000_000), "2026-11-06T21:20:00Z");
        assert_eq!(hms(1_794_000_000), "21:20:00");
    }
}
