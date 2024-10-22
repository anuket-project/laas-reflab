use anyhow::{Error, Result};
use serde::{Deserialize, Serialize};
use tokio_postgres::types::{private::BytesMut, FromSql, IsNull, ToSql, Type};

use std::{fmt, str::FromStr};

use common::BoxedError;

mod nxos;
mod sonic;

pub use nxos::NxosVersion;
pub use sonic::SonicVersion;

#[derive(Serialize, Deserialize, Debug, Clone, Eq, PartialEq)]
pub enum Version {
    Nxos(NxosVersion),
    Sonic(SonicVersion),
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Version::Nxos(nxos_version) => write!(f, "{}", nxos_version),
            Version::Sonic(sonic_version) => write!(f, "{}", sonic_version),
        }
    }
}

impl FromStr for Version {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.contains('.') && s.contains('(') && s.contains(')') {
            let nxos_version = NxosVersion::from_str(s)?;
            Ok(Version::Nxos(nxos_version))
        } else if s.len() == 6 {
            let sonic_version = SonicVersion::from_str(s)?;
            Ok(Version::Sonic(sonic_version))
        } else {
            anyhow::bail!("version format incorrect");
        }
    }
}

impl ToSql for Version {
    fn to_sql(&self, ty: &Type, out: &mut BytesMut) -> Result<IsNull, BoxedError>
    where
        Self: Sized,
    {
        self.to_string().to_sql(ty, out)
    }

    fn to_sql_checked(&self, ty: &Type, out: &mut BytesMut) -> Result<IsNull, BoxedError> {
        self.to_string().to_sql_checked(ty, out)
    }

    fn accepts(ty: &Type) -> bool
    where
        Self: Sized,
    {
        <String as ToSql>::accepts(ty)
    }
}

impl FromSql<'_> for Version {
    fn from_sql(ty: &Type, raw: &[u8]) -> Result<Self, BoxedError> {
        // convert the raw bytes into a `String`
        let s = String::from_sql(ty, raw)?;

        // parse the `String` into a `Version`
        Version::from_str(&s).map_err(|e| e.into())
    }

    fn accepts(ty: &Type) -> bool {
        <String as FromSql>::accepts(ty)
    }
}
