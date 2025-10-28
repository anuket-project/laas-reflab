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
    JsonSchema,
)]
#[sqlx(type_name = "storage_type", rename_all = "lowercase")]
#[serde(rename_all = "lowercase")]
pub enum StorageType {
    #[strum(serialize = "ssd")]
    #[sqlx(rename = "ssd")]
    SSD,
    #[strum(serialize = "hdd")]
    #[sqlx(rename = "hdd")]
    HDD,
    #[strum(serialize = "hybrid")]
    #[sqlx(rename = "hybrid")]
    Hybrid,
}

// This is another example of something we only need while partially depending on the `dal`
// TODO: Delete when [`dal`] is deprecated
impl FromSql<'_> for StorageType {
    fn from_sql(_ty: &Type, raw: &[u8]) -> Result<Self, Box<dyn Error + Sync + Send>> {
        let s = std::str::from_utf8(raw)?;
        match s {
            "ssd" => Ok(StorageType::SSD),
            "hdd" => Ok(StorageType::HDD),
            "hybrid" => Ok(StorageType::Hybrid),
            other => Err(format!("Invalid StorageType enum variant: {}", other).into()),
        }
    }

    fn accepts(ty: &Type) -> bool {
        ty.name() == "storage_type"
    }
}

// This is another example of something we only need while partially depending on the `dal`
// TODO: Delete when [`dal`] is deprecated
impl ToSql for StorageType {
    fn to_sql(
        &self,
        _ty: &Type,
        out: &mut BytesMut,
    ) -> Result<tokio_postgres::types::IsNull, Box<dyn Error + Sync + Send>> {
        let s = match self {
            StorageType::SSD => "ssd",
            StorageType::HDD => "hdd",
            StorageType::Hybrid => "hybrid",
        };
        out.extend_from_slice(s.as_bytes());
        Ok(tokio_postgres::types::IsNull::No)
    }

    fn accepts(ty: &Type) -> bool {
        ty.name() == "storage_type"
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

    impl Arbitrary for StorageType {
        type Parameters = ();
        type Strategy = BoxedStrategy<Self>;

        fn arbitrary_with(_: Self::Parameters) -> Self::Strategy {
            prop_oneof![
                Just(StorageType::SSD),
                Just(StorageType::HDD),
                Just(StorageType::Hybrid),
            ]
            .boxed()
        }
    }
}
