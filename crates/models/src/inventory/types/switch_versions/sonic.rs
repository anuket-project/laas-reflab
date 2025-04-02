use anyhow::{Error, Result};
use serde::{Deserialize, Serialize};
use std::{fmt, str::FromStr};

#[derive(Serialize, Deserialize, Debug, Clone, Eq, PartialEq)]
pub struct SonicVersion {
    year: u16, // 0-9999 (covers all reasonable calendar years)
    month: u8, // 1-12
}

impl fmt::Display for SonicVersion {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{:04}{:02}", self.year, self.month)
    }
}

impl FromStr for SonicVersion {
    type Err = Error;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        // validate length first
        if value.len() != 6 {
            return Err(Error::msg("Version must be 6 characters in YYYYMM format"));
        }

        let (year_str, month_str) = value.split_at(4);

        // parse year with range check
        let year = year_str
            .parse::<u16>()
            .map_err(|_| Error::msg("Year must be between 0000-9999"))?;

        // parse month with range check
        let month = month_str
            .parse::<u8>()
            .map_err(|_| Error::msg("Invalid month format"))?;

        if !(1..=12).contains(&month) {
            return Err(Error::msg("Month must be between 01-12"));
        }

        Ok(SonicVersion { year, month })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use proptest::prelude::*;

    impl Arbitrary for SonicVersion {
        type Parameters = ();
        type Strategy = BoxedStrategy<Self>;

        fn arbitrary_with(_: Self::Parameters) -> Self::Strategy {
            (
                0..=9999u16, // Valid year range
                1..=12u8,    // Valid month range
            )
                .prop_map(|(year, month)| SonicVersion { year, month })
                .boxed()
        }
    }

    #[test]
    fn valid_round_trip() {
        let v = SonicVersion {
            year: 2023,
            month: 12,
        };
        let s = v.to_string();
        assert_eq!(s, "202312");
        let parsed: SonicVersion = s.parse().unwrap();
        assert_eq!(v, parsed);
    }

    #[test]
    fn leading_zero_years() {
        let v = SonicVersion { year: 23, month: 4 };
        assert_eq!(v.to_string(), "002304");
        let parsed: SonicVersion = "002304".parse().unwrap();
        assert_eq!(v, parsed);
    }

    #[test]
    fn invalid_length() {
        assert!("2023".parse::<SonicVersion>().is_err());
        assert!("2023105".parse::<SonicVersion>().is_err());
    }

    #[test]
    fn invalid_month() {
        assert!("202300".parse::<SonicVersion>().is_err());
        assert!("202313".parse::<SonicVersion>().is_err());
    }

    #[test]
    fn invalid_year_chars() {
        assert!("20x312".parse::<SonicVersion>().is_err());
    }
}
