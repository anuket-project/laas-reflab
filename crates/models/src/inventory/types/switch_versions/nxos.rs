use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;
use std::str::Split;

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct NxosVersion {
    major: i32,
    minor: i32,
    maintenance: i32,
    train: String,
    rebuild: i32,
}

impl fmt::Display for NxosVersion {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "{}.{}({}){}{}",
            self.major, self.minor, self.maintenance, self.train, self.rebuild
        )
    }
}

impl FromStr for NxosVersion {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if !s.contains('.') {
            bail!("version format incorrect")
        }

        // 7.0(3)I3(1)
        let mut version_nums = s.split(['.', '(', ')']);
        // 7 0 3 I3 1
        let major = version_num_from_split(&mut version_nums)?;
        let minor = version_num_from_split(&mut version_nums)?;
        let maintenance = version_num_from_split(&mut version_nums)?;

        let mut train = String::new();
        let mut rebuild = 0;
        match version_nums.next() {
            Some(st) => {
                if st.len() == 2 {
                    let temp = st.split_at(1);
                    train = temp.0.to_string();
                    rebuild = temp.1.parse()?
                } else if st.len() == 1 {
                    train = st.to_string();
                    rebuild = 0;
                }
            }
            None => {
                train = "".to_string();
                rebuild = 0;
            }
        };
        Ok(NxosVersion {
            major,
            minor,
            maintenance,
            train,
            rebuild,
        })
    }
}

fn version_num_from_split(spl: &mut Split<'_, [char; 3]>) -> Result<i32> {
    match spl.next() {
        Some(st) => st
            .parse()
            .map_err(|_| anyhow::anyhow!("failed to parse version number")),
        None => bail!("version format incorrect"),
    }
}
