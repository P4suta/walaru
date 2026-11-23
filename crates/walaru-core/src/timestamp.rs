use std::{fmt, ops::Sub, str::FromStr};

use time::{SignedDuration, UtcDateTime, format_description::well_known::Rfc3339};

/// A run instant in UTC, including inserted seconds recorded in existing databases.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RunTimestamp {
    utc: UtcDateTime,
    leap_second: bool,
}

impl RunTimestamp {
    /// Reads the system clock in UTC.
    #[must_use]
    pub fn now() -> Self {
        UtcDateTime::now().into()
    }
}

impl From<UtcDateTime> for RunTimestamp {
    fn from(utc: UtcDateTime) -> Self {
        Self {
            utc,
            leap_second: false,
        }
    }
}

impl FromStr for RunTimestamp {
    type Err = time::error::Parse;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let leap_second = value
            .get(..19)
            .is_some_and(|prefix| prefix.is_ascii() && prefix.ends_with("60"));
        let normalized;
        let input = if leap_second || value.contains('−') {
            let mut text = value.replace('−', "-");
            if leap_second {
                text.replace_range(17..19, "59");
            }
            normalized = text;
            normalized.as_str()
        } else {
            value
        };
        let utc = UtcDateTime::parse(input, &Rfc3339)?;
        Ok(Self { utc, leap_second })
    }
}

impl fmt::Display for RunTimestamp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let instant = self.utc;
        let second = if self.leap_second {
            60
        } else {
            instant.second()
        };
        write!(
            f,
            "{}T{:02}:{:02}:{second:02}",
            instant.date(),
            instant.hour(),
            instant.minute()
        )?;
        let nanos = instant.nanosecond();
        if nanos != 0 {
            if nanos.is_multiple_of(1_000_000) {
                write!(f, ".{:03}", nanos / 1_000_000)?;
            } else if nanos.is_multiple_of(1_000) {
                write!(f, ".{:06}", nanos / 1_000)?;
            } else {
                write!(f, ".{nanos:09}")?;
            }
        }
        f.write_str("+00:00")
    }
}

impl Sub<SignedDuration> for RunTimestamp {
    type Output = Self;

    fn sub(self, age: SignedDuration) -> Self {
        let mut cutoff = self.utc - age;
        if self.leap_second {
            if cutoff.unix_timestamp() == self.utc.unix_timestamp() {
                return Self {
                    utc: cutoff,
                    leap_second: true,
                };
            }
            if cutoff < self.utc {
                cutoff += SignedDuration::seconds(1);
            }
        }
        cutoff.into()
    }
}
