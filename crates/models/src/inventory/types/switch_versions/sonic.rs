use anyhow::{Error, Result};
use serde::{Deserialize, Serialize};
use std::{fmt, str::FromStr};

#[derive(Serialize, Deserialize, Debug, Clone, Eq, PartialEq)]
pub struct SonicVersion {
    year: i32,
    month: i8, // If there are somehow more than 256 months in a year please add this to the list of falsehoods programmers believe about time
}

impl fmt::Display for SonicVersion {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}{:02}", self.year, self.month)
    }
}

impl FromStr for SonicVersion {
    type Err = Error;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let version_nums = value.split_at(4);
        Ok(SonicVersion {
            year: version_nums.0.parse()?,
            month: version_nums.1.parse()?,
        })
    }
}
