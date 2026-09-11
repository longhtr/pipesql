//! Validated Gregorian dates shared by import, binding and result exchange.
/// A Gregorian date in years 0001 through 9999, measured from 1970-01-01.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct DateValue {
    days: i32,
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum DatePart {
    Day,
    Month,
    Year,
}

impl DateValue {
    /// Construct a date in years 0001..=9999 from a signed day offset from 1970-01-01.
    /// Returns `None` outside that calendar range.
    pub fn from_days_since_unix_epoch(days: i32) -> Option<Self> {
        Self::from_days(days)
    }

    /// Signed days from 1970-01-01. Earlier dates have negative offsets.
    pub fn days_since_unix_epoch(self) -> i32 {
        self.days
    }

    pub(crate) fn from_days(days: i32) -> Option<Self> {
        (-719_162..=2_932_896)
            .contains(&days)
            .then_some(Self { days })
    }

    pub(crate) fn parse(bytes: &[u8]) -> Option<Self> {
        if bytes.len() != 10 || bytes[4] != b'-' || bytes[7] != b'-' {
            return None;
        }
        let digits = |bytes: &[u8]| {
            bytes.iter().try_fold(0_i64, |n, b| {
                b.is_ascii_digit().then(|| n * 10 + i64::from(*b - b'0'))
            })
        };
        Self::civil(
            digits(&bytes[..4])?,
            digits(&bytes[5..7])?,
            digits(&bytes[8..])?,
        )
    }

    fn civil(year: i64, month: i64, day: i64) -> Option<Self> {
        if !(1..=9999).contains(&year)
            || !(1..=12).contains(&month)
            || day < 1
            || day > month_days(year, month)
        {
            return None;
        }
        // Validated calendar fields bound every intermediate to a few million.
        let adjusted = year - i64::from(month <= 2);
        let era = adjusted.div_euclid(400);
        let y = adjusted - era * 400;
        let m = month + if month > 2 { -3 } else { 9 };
        let days = era * 146097 + y * 365 + y / 4 - y / 100 + (153 * m + 2) / 5 + day - 1 - 719468;
        Self::from_days(i32::try_from(days).ok()?)
    }

    fn components(self) -> (i64, i64, i64) {
        let shifted = i64::from(self.days) + 719468;
        let era = shifted.div_euclid(146097);
        let d = shifted - era * 146097;
        let y = (d - d / 1460 + d / 36524 - d / 146096) / 365;
        let day = d - (365 * y + y / 4 - y / 100);
        let m = (5 * day + 2) / 153;
        let month = m + if m < 10 { 3 } else { -9 };
        (
            y + era * 400 + i64::from(month <= 2),
            month,
            day - (153 * m + 2) / 5 + 1,
        )
    }

    pub(crate) fn shift(self, interval: i64, part: DatePart) -> Option<Self> {
        if matches!(part, DatePart::Day) {
            return Self::from_days(
                i32::try_from(i64::from(self.days).checked_add(interval)?).ok()?,
            );
        }
        let (year, month, day) = self.components();
        let months = if matches!(part, DatePart::Year) {
            interval.checked_mul(12)?
        } else {
            interval
        };
        let shifted = (year * 12 + month - 1).checked_add(months)?;
        let year = shifted.div_euclid(12);
        let month = shifted.rem_euclid(12) + 1;
        Self::civil(year, month, day.min(month_days(year, month)))
    }
}

fn month_days(year: i64, month: i64) -> i64 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => 28 + i64::from(year % 4 == 0 && (year % 100 != 0 || year % 400 == 0)),
        _ => unreachable!("validated month"),
    }
}

impl std::fmt::Display for DateValue {
    fn fmt(&self, output: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let (year, month, day) = self.components();
        write!(output, "{year:04}-{month:02}-{day:02}")
    }
}
