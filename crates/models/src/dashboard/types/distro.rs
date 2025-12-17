use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::error::Error;
use strum_macros::{Display, EnumString};
use tokio_postgres::types::{private::BytesMut, FromSql, ToSql, Type};

#[derive(
    Serialize,
    Deserialize,
    Clone,
    Debug,
    Hash,
    Copy,
    EnumString,
    Display,
    Eq,
    PartialEq,
    Default,
    sqlx::Type,
    JsonSchema,
)]
#[sqlx(type_name = "distro")]
pub enum Distro {
    #[default]
    #[strum(serialize = "Ubuntu")]
    Ubuntu,
    #[strum(serialize = "Fedora")]
    Fedora,
    #[strum(serialize = "Alma")]
    Alma,
    #[strum(serialize = "EVE")]
    #[serde(rename = "EVE")]
    #[sqlx(rename = "EVE")]
    Eve,
}

// This is another example of something we only need while partially depending on the `dal`
impl FromSql<'_> for Distro {
    fn from_sql(_ty: &Type, raw: &[u8]) -> Result<Self, Box<dyn Error + Sync + Send>> {
        let s = std::str::from_utf8(raw)?;
        match s {
            "Ubuntu" => Ok(Distro::Ubuntu),
            "Fedora" => Ok(Distro::Fedora),
            "Alma" => Ok(Distro::Alma),
            "EVE" => Ok(Distro::Eve),
            other => Err(format!("Invalid Distro enum variant: {}", other).into()),
        }
    }

    fn accepts(ty: &Type) -> bool {
        ty.name() == "distro"
    }
}

// This is another example of something we only need while partially depending on the `dal`
// TODO: Delete when [`dal`] is deprecated
impl ToSql for Distro {
    fn to_sql(
        &self,
        _ty: &Type,
        out: &mut BytesMut,
    ) -> Result<tokio_postgres::types::IsNull, Box<dyn Error + Sync + Send>> {
        let s = match self {
            Distro::Ubuntu => "Ubuntu",
            Distro::Fedora => "Fedora",
            Distro::Alma => "Alma",
            Distro::Eve => "EVE",
        };
        out.extend_from_slice(s.as_bytes());
        Ok(tokio_postgres::types::IsNull::No)
    }

    fn accepts(ty: &Type) -> bool {
        ty.name() == "distro"
    }

    fn to_sql_checked(
        &self,
        ty: &Type,
        out: &mut BytesMut,
    ) -> Result<tokio_postgres::types::IsNull, Box<dyn Error + Sync + Send>> {
        if !<Self as ToSql>::accepts(ty) {
            return Err(format!("cannot convert to type {}", ty.name()).into());
        }
        self.to_sql(ty, out)
    }
}
