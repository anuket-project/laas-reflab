use anyhow::{anyhow, Result};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct NxosVersion {
    major: u32,
    minor: u32,
    maintenance: u32,
    train: Option<String>,
    rebuild: Option<u32>,
}

impl fmt::Display for NxosVersion {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        if let Some(train) = &self.train {
            if let Some(rebuild) = self.rebuild {
                write!(
                    f,
                    "{}.{}({}){}({})",
                    self.major, self.minor, self.maintenance, train, rebuild
                )
            } else {
                write!(
                    f,
                    "{}.{}({}){}",
                    self.major, self.minor, self.maintenance, train
                )
            }
        } else {
            write!(f, "{}.{}({})", self.major, self.minor, self.maintenance)
        }
    }
}

impl FromStr for NxosVersion {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let re = Regex::new(r"^(\d+)\.(\d+)\((\d+)\)([A-Z][A-Z0-9]*)?(?:\((\d+)\))?$")
            .map_err(|e| anyhow!("Invalid regex pattern: {}", e))?;

        let caps = re.captures(s).ok_or_else(|| {
            anyhow!("Invalid NX-OS version format. Expected format: MAJOR.MINOR(MAINTENANCE)TRAIN(REBUILD)")
        })?;

        Ok(NxosVersion {
            major: caps[1]
                .parse()
                .map_err(|e| anyhow!("Invalid major version: {}", e))?,
            minor: caps[2]
                .parse()
                .map_err(|e| anyhow!("Invalid minor version: {}", e))?,
            maintenance: caps[3]
                .parse()
                .map_err(|e| anyhow!("Invalid maintenance version: {}", e))?,
            train: caps.get(4).map(|m| m.as_str().to_string()),
            rebuild: caps.get(5).and_then(|m| m.as_str().parse().ok()),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use proptest::prelude::*;
    use proptest::string::string_regex;

    impl Arbitrary for NxosVersion {
        type Parameters = ();
        type Strategy = BoxedStrategy<Self>;

        fn arbitrary_with(_: Self::Parameters) -> Self::Strategy {
            let train_strat = string_regex("[A-Z][A-Z0-9]{0,9}").unwrap();

            (0u32..9999, 0u32..9999, 0u32..9999, train_strat, 0u32..9999)
                .prop_map(|(major, minor, maintenance, train, rebuild)| NxosVersion {
                    major,
                    minor,
                    maintenance,
                    train,
                    rebuild,
                })
                .boxed()
        }
    }

    #[test]
    fn valid_version_roundtrip() {
        let version = "7.0(3)I3(1)";
        let parsed: NxosVersion = version.parse().unwrap();
        assert_eq!(parsed.to_string(), version);
    }

    #[test]
    fn invalid_formats() {
        assert!("7.0(3)I3".parse::<NxosVersion>().is_err());
        assert!("7.0(3)I3()".parse::<NxosVersion>().is_err());
        assert!("7.0(3)i3(1)".parse::<NxosVersion>().is_err());
        assert!("7.0.3(I3)(1)".parse::<NxosVersion>().is_err());
    }

    #[test]
    fn edge_cases() {
        let max_version = "9999.9999(9999)ZZZZ9999(9999)";
        let parsed: NxosVersion = max_version.parse().unwrap();
        assert_eq!(parsed.to_string(), max_version);
    }
}

