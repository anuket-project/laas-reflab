use tokio_postgres::types::ToSql;

use common::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::to_value;
use serde_json::Value;
use tokio_postgres::types::{private::BytesMut, IsNull, Type};

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LifeCycleState {
    #[default]
    New, // signals this booking has not yet been fully provisioned
    Active, // signals this booking is actively being used and has already been provisioned
    // (ready for cleanup, if it's time)
    Done, // signals this booking has been cleaned up and released
}

type BoxedError = Box<dyn std::error::Error + Sync + Send>;

impl ToSql for LifeCycleState {
    fn to_sql(&self, ty: &Type, out: &mut BytesMut) -> Result<IsNull, BoxedError>
    where
        Self: Sized,
    {
        to_value(self)?.to_sql(ty, out)
    }

    fn accepts(ty: &Type) -> bool
    where
        Self: Sized,
    {
        <Value as ToSql>::accepts(ty)
    }

    fn to_sql_checked(&self, ty: &Type, out: &mut BytesMut) -> Result<IsNull, BoxedError> {
        serde_json::to_value(self)?.to_sql_checked(ty, out)
    }
}

impl std::fmt::Display for LifeCycleState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        <Self as std::fmt::Debug>::fmt(self, f)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    impl Arbitrary for LifeCycleState {
        type Parameters = ();
        type Strategy = BoxedStrategy<Self>;

        fn arbitrary_with(_args: Self::Parameters) -> Self::Strategy {
            prop_oneof![
                Just(LifeCycleState::New),
                Just(LifeCycleState::Active),
                Just(LifeCycleState::Done),
            ]
            .boxed()
        }
    }
}
