//! We need this module to tell SQLx how to encode/decode our FKey<T> type.
//!
//! Since normally SQLx would allow us to just use the Uuid type directly on the `id` field in our structs,
//! this would be unnecessary if we didn't rely on our current custom ORM and DBTable trait in workflows
//! and other various places. If we ever fully migrate to SQLx this can be removed.

use sqlx::encode::IsNull;
use sqlx::postgres::{PgTypeInfo, PgValueRef};
use sqlx::{Database, Decode, Encode, Postgres, Type};
use uuid::Uuid;

use super::{DBTable, FKey, ID};

/// Tells SQLx that our [`FKey<T>`] should be treated in SQL as a Postgres `UUID`.
///
/// See [`Type`] docs for more details.
impl<T: DBTable> Type<Postgres> for FKey<T> {
    /// Return the Postgres type information for `UUID`.
    fn type_info() -> PgTypeInfo {
        <Uuid as Type<Postgres>>::type_info()
    }
}

/// Decode a native Postgres UUID into our [`FKey<T>`].
///
/// 1. Decode the value as [`Uuid`].
/// 2. Wrap the [`Uuid`] inside our [`FKey<T>`] (via [`ID`]).
impl<'r, T: DBTable> Decode<'r, Postgres> for FKey<T> {
    fn decode(value: PgValueRef<'r>) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        // decode the raw bytes as a UUID.
        let uuid: Uuid = Decode::decode(value)?;
        // wrap the UUID into our FKey using the ID newtype.
        // TODO: ID should be axed
        Ok(FKey::from_id(ID::from(uuid)))
    }
}

/// Encode an [`FKey<T>`] into native Postgres `UUID` type.
///
/// Serialize our [`FKey<T>`] into a Postgres query parameter by delegating to the
/// existing `Uuid::Encode` implementation.
impl<'q, T: DBTable> Encode<'q, Postgres> for FKey<T> {
    /// write the argument to the Postgres buffer by reusing Uuid’s encoder.
    fn encode_by_ref(
        &self,
        buf: &mut <Postgres as Database>::ArgumentBuffer<'q>,
    ) -> Result<IsNull, Box<dyn std::error::Error + Send + Sync>> {
        // delegate the actual bytes to Uuid‘s Encode implementation.
        <Uuid as Encode<'q, Postgres>>::encode(self.id.0, buf)
    }

    /// optional method to provide what SQL type this parameter produces (UUID).
    /// See [`Encode::produces`].
    fn produces(&self) -> Option<<Postgres as Database>::TypeInfo> {
        Some(<Uuid as Type<Postgres>>::type_info())
    }

    /// optional method to provide a size hint for prepared-statement optimizations.
    /// See [`Encode::size_hint`].
    fn size_hint(&self) -> usize {
        <Uuid as Encode<'q, Postgres>>::size_hint(&self.id.0)
    }
}
