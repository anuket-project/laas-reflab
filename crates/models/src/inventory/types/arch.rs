use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::error::Error;
use strum_macros::{Display, EnumString};
use tokio_postgres::types::{private::BytesMut, FromSql, ToSql, Type};

#[derive(
    sqlx::Type,
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
    JsonSchema,
)]
#[sqlx(type_name = "arch", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum Arch {
    #[default]
    #[strum(serialize = "x86_64")]
    #[serde(rename = "x86_64")]
    #[sqlx(rename = "x86_64")]
    X86_64,
    #[strum(serialize = "aarch64")]
    #[serde(rename = "aarch64")]
    #[sqlx(rename = "aarch64")]
    Aarch64,
}

impl Arch {
    pub fn from_string_fuzzy(s: &str) -> Option<Arch> {
        if s.contains("x86_64") {
            Some(Arch::X86_64)
        } else if s.contains("aarch64") {
            Some(Arch::Aarch64)
        } else {
            None
        }
    }
}

// This is another example of something we only need while partially depending on the `dal`
// TODO: Delete when [`dal`] is deprecated
impl FromSql<'_> for Arch {
    fn from_sql(_ty: &Type, raw: &[u8]) -> Result<Self, Box<dyn Error + Sync + Send>> {
        let s = std::str::from_utf8(raw)?;
        match s {
            "x86_64" => Ok(Arch::X86_64),
            "aarch64" => Ok(Arch::Aarch64),
            other => Err(format!("Invalid Arch enum variant: {}", other).into()),
        }
    }

    fn accepts(ty: &Type) -> bool {
        ty.name() == "arch"
    }
}

// This is another example of something we only need while partially depending on the `dal`
// TODO: Delete when [`dal`] is deprecated
impl ToSql for Arch {
    fn to_sql(
        &self,
        _ty: &Type,
        out: &mut BytesMut,
    ) -> Result<tokio_postgres::types::IsNull, Box<dyn Error + Sync + Send>> {
        let s = match self {
            Arch::X86_64 => "x86_64",
            Arch::Aarch64 => "aarch64",
        };
        out.extend_from_slice(s.as_bytes());
        Ok(tokio_postgres::types::IsNull::No)
    }

    fn accepts(ty: &Type) -> bool {
        ty.name() == "arch"
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

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    impl Arbitrary for Arch {
        type Parameters = ();
        type Strategy = BoxedStrategy<Self>;

        fn arbitrary_with(_: Self::Parameters) -> Self::Strategy {
            prop_oneof![Just(Arch::X86_64), Just(Arch::Aarch64),].boxed()
        }
    }
}
