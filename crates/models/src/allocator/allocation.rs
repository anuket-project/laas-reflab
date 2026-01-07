use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use dal::{web::*, *};

use crate::{allocator::ResourceHandle, dashboard::Aggregate, inventory::*};

#[derive(Serialize, Deserialize, Debug, Clone, Eq, PartialEq)]
pub struct Allocation {
    pub id: FKey<Allocation>,
    pub for_resource: FKey<ResourceHandle>,
    pub for_aggregate: Option<FKey<Aggregate>>,
    pub started: DateTime<Utc>,
    pub ended: Option<DateTime<Utc>>,
    pub reason_started: AllocationReason,
    pub reason_ended: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, Eq, PartialEq, JsonSchema)]
pub enum AllocationReason {
    /// If a resource is to be used within a booking, allocate
    /// with ForBooking
    #[serde(rename = "booking")]
    ForBooking,

    /// If a resource is being temporarily taken out of
    /// commission for downtime of some sort,
    /// it should be allocated as ForMaintenance
    #[serde(rename = "maintenance")]
    ForMaintenance,

    /// If a resource is being taken out of commission,
    /// it should be allocated with reason ForRetiry
    #[serde(rename = "retire")]
    ForRetiry,
}

#[derive(Serialize, Deserialize, Debug, Clone, Hash, Copy)]
pub enum AllocationStatus {
    Allocated,
    Free,
    Broken,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy)]
pub enum AllocationOperation {
    /// This operation gives out the related handle to be used by a user
    Allocate,

    /// This operation takes back the related handle and returns it to
    /// the available pool
    Release,
}

impl DBTable for Allocation {
    fn id(&self) -> ID {
        self.id.into_id()
    }

    fn id_mut(&mut self) -> &mut ID {
        self.id.into_id_mut()
    }

    fn table_name() -> &'static str {
        "allocations"
    }

    fn from_row(row: tokio_postgres::Row) -> Result<ExistingRow<Self>, anyhow::Error> {
        Ok(ExistingRow::from_existing(Self {
            id: row.try_get("id")?,
            for_resource: row.try_get("for_resource")?,
            for_aggregate: row.try_get("for_aggregate")?,
            started: row.try_get("started")?,
            ended: row.try_get("ended")?,

            reason_started: serde_json::from_str(row.try_get("reason_started")?)?,
            reason_ended: row.try_get("reason_ended")?,
        }))
    }

    fn to_rowlike(&self) -> Result<HashMap<&str, Box<dyn ToSqlObject>>, anyhow::Error> {
        let c: [(&str, Box<dyn ToSqlObject>); _] = [
            ("id", Box::new(self.id)),
            ("for_resource", Box::new(self.for_resource)),
            ("for_aggregate", Box::new(self.for_aggregate)),
            ("started", Box::new(self.started)),
            ("ended", Box::new(self.ended)),
            (
                "reason_started",
                Box::new(serde_json::to_string(&self.reason_started)?),
            ),
            ("reason_ended", Box::new(self.reason_ended.clone())),
        ];

        Ok(c.into_iter().collect())
    }
}

impl Allocation {
    pub fn new(
        id: FKey<Allocation>,
        for_resource: FKey<ResourceHandle>,
        for_aggregate: Option<FKey<Aggregate>>,
        started: DateTime<Utc>,
        ended: Option<DateTime<Utc>>,
        reason_started: AllocationReason,
        reason_ended: Option<String>,
    ) -> Self {
        Self {
            id,
            for_resource,
            for_aggregate,
            started,
            ended,
            reason_started,
            reason_ended,
        }
    }

    pub async fn find(
        t: &mut EasyTransaction<'_>,
        for_resource: FKey<ResourceHandle>,
        completed: bool,
    ) -> Result<Vec<ExistingRow<Allocation>>, anyhow::Error> {
        let tn = Self::table_name();
        let q = if completed {
            format!("SELECT * FROM {tn} WHERE ended IS NOT NULL AND for_resource = $1")
        } else {
            format!("SELECT * FROM {tn} WHERE ended IS NULL AND for_resource = $1")
        };

        let rows = t.query(&q, &[&for_resource]).await.anyway()?;

        Allocation::from_rows(rows)
    }

    /// Returns the most recent allocation for a resource based on started time.
    /// Returns None if the resource has never been allocated
    /// Returns an error if the DB lookup failed
    pub async fn get_most_recent_allocation_for_resource(
        t: &mut EasyTransaction<'_>,
        for_resource: FKey<ResourceHandle>,
    ) -> Result<Option<Allocation>, anyhow::Error> {
        let query = format!(
            "SELECT * FROM {} where for_resource = $1 ORDER BY started DESC LIMIT 1",
            Self::table_name()
        );
        let rows = t.query(&query, &[&for_resource]).await.anyway()?;

        match Allocation::from_rows(rows)?.first() {
            Some(row) => Ok(Some(row.clone().into_inner())),
            None => Ok(None),
        }
    }

    pub async fn all_for_aggregate(
        t: &mut EasyTransaction<'_>,
        agg: FKey<Aggregate>,
    ) -> Result<Vec<ExistingRow<Allocation>>, anyhow::Error> {
        let tn = Self::table_name();

        let q = format!("SELECT * FROM {tn} WHERE for_aggregate = $1;");

        let rows = t.query(&q, &[&Some(agg)]).await.anyway()?;
        Self::from_rows(rows)
    }

    /// Finds allocations given an aggregate, host, and whether or not it is active.
    pub async fn find_for_aggregate_and_host(
        t: &mut EasyTransaction<'_>,
        agg: FKey<Aggregate>,
        host: FKey<Host>,
        completed: bool,
    ) -> Result<Vec<ExistingRow<Allocation>>, anyhow::Error> {
        let allocation_tn = Self::table_name();
        let rh_tn = ResourceHandle::table_name();

        let rh_id = format!("select {rh_tn}.id from {rh_tn} where tracks_resource = $2");
        let is_null = match completed {
            true => "is not null",
            false => "is null",
        };

        let q = format!("select * from {allocation_tn} where for_aggregate = $1 and ended {is_null} and for_resource = ({rh_id})");

        let rows = t.query(&q, &[&agg, &host]).await.anyway()?;

        Allocation::from_rows(rows)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::option::of;
    use proptest::prelude::*;
    use testing_utils::{block_on_runtime, datetime_utc_strategy, insert_default_model_at};

    impl Arbitrary for AllocationReason {
        type Parameters = ();
        type Strategy = BoxedStrategy<Self>;

        fn arbitrary_with(_args: Self::Parameters) -> Self::Strategy {
            prop_oneof![
                Just(AllocationReason::ForBooking),
                Just(AllocationReason::ForMaintenance),
                Just(AllocationReason::ForRetiry),
            ]
            .boxed()
        }
    }

    impl Arbitrary for Allocation {
        type Parameters = ();
        type Strategy = BoxedStrategy<Self>;

        fn arbitrary_with(_args: Self::Parameters) -> Self::Strategy {
            (
                any::<FKey<Allocation>>(),
                any::<FKey<ResourceHandle>>(),
                of(any::<FKey<Aggregate>>()),
                datetime_utc_strategy(),
                of(datetime_utc_strategy()),
                any::<AllocationReason>(),
                of("[a-zA-Z0-9]{1,20}"),
            )
                .prop_map(
                    |(
                        id,
                        for_resource,
                        for_aggregate,
                        started,
                        ended,
                        reason_started,
                        reason_ended,
                    )| Allocation {
                        id,
                        for_resource,
                        for_aggregate,
                        started,
                        ended,
                        reason_started,
                        reason_ended,
                    },
                )
                .boxed()
        }
    }

    proptest! {
        #[test]
        fn test_allocation_model(allocation in Allocation::arbitrary()) {
            block_on_runtime!({
                let client = new_client().await;
                prop_assert!(client.is_ok(), "DB connection failed: {:?}", client.err());
                let mut client = client.unwrap();

                let transaction = client.easy_transaction().await;
                prop_assert!(transaction.is_ok(), "Transaction creation failed: {:?}", transaction.err());
                let mut transaction = transaction.unwrap();

                let resource_handle_insert_result = insert_default_model_at(allocation.for_resource, &mut transaction).await;
                prop_assert!(resource_handle_insert_result.is_ok(), "Failed to prepare test environment: {:?}", resource_handle_insert_result.err());

                if let Some(agg) = allocation.for_aggregate {
                    let aggregate_insert_result = Aggregate::insert_default_at(agg, &mut transaction).await;
                    prop_assert!(aggregate_insert_result.is_ok(), "Failed to prepare test environment: {:?}", aggregate_insert_result.err());
                }


                let new_row = NewRow::new(allocation.clone());
                let insert_result = new_row.insert(&mut transaction).await;
                prop_assert!(insert_result.is_ok(), "Insert failed: {:?}", insert_result.err());

                let retrieved_allocation = Allocation::select()
                    .where_field("id")
                    .equals(allocation.id)
                    .run(&mut transaction)
                    .await;

                prop_assert!(retrieved_allocation.is_ok(), "Retrieval failed: {:?}", retrieved_allocation.err());
                let retrieved_allocation_vec = retrieved_allocation.unwrap();

                let first_allocation = retrieved_allocation_vec.first();
                prop_assert!(first_allocation.is_some(), "No Allocation found, empty result");

                let retrieved_allocation = first_allocation.unwrap().clone().into_inner();
                prop_assert_eq!(retrieved_allocation, allocation);

                Ok(())

            })?
        }

        #[test]
        /// Tests [`Allocation::find_for_aggregate_and_host`]
        /// Proptest ensures a mix of ended and not ended allocations.
        fn test_find_allocation_for_aggregate_and_host(allocation in Allocation::arbitrary()) {
            block_on_runtime!({

                let client = new_client().await;
                prop_assert!(client.is_ok(), "DB connection failed: {:?}", client.err());
                let mut client = client.unwrap();

                let transaction = client.easy_transaction().await;
                prop_assert!(transaction.is_ok(), "Transaction creation failed: {:?}", transaction.err());
                let mut transaction = transaction.unwrap();

                let resource_handle_insert_result = insert_default_model_at(allocation.for_resource, &mut transaction).await;
                prop_assert!(resource_handle_insert_result.is_ok(), "Failed to prepare test environment: {:?}", resource_handle_insert_result.err());

                if let Some(agg) = allocation.for_aggregate {
                    let aggregate_insert_result = Aggregate::insert_default_at(agg, &mut transaction).await;
                    prop_assert!(aggregate_insert_result.is_ok(), "Failed to prepare test environment: {:?}", aggregate_insert_result.err());
                }

                let new_row = NewRow::new(allocation.clone());
                let insert_result = new_row.insert(&mut transaction).await;
                prop_assert!(insert_result.is_ok(), "Insert failed: {:?}", insert_result.err());

                let rh = ResourceHandle::select().where_field("id").equals(allocation.for_resource).run(&mut transaction).await.unwrap().first().unwrap().clone();

                if let Some(_) = allocation.for_aggregate {
                    if let crate::allocator::ResourceHandleInner::Host(host) = rh.tracks {
                        let queried_allocation = Allocation::find_for_aggregate_and_host(&mut transaction, allocation.for_aggregate.unwrap(), host, allocation.ended.is_some()).await.unwrap().first().unwrap().clone().into_inner();
                        prop_assert_eq!(queried_allocation, allocation);
                    } else {
                        prop_assert!(false, "RH isn't tracking a host?");
                    }
                }

                Ok(())

            })?
        }
    }
}
