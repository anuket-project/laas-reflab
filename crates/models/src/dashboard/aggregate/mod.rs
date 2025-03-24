use common::prelude::{
    chrono::{DateTime, Utc},
    *,
};
use dal::*;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

mod lifecycle_state;
pub use lifecycle_state::LifeCycleState;

use crate::{
    dashboard::{Instance, NetworkAssignmentMap, Template},
    inventory::Lab,
};

#[derive(Serialize, Deserialize, Debug, Clone, Default, Eq, PartialEq)]
pub struct BookingMetadata {
    /// The dashboard booking id
    pub booking_id: Option<String>,
    /// The ipa username of the owner of the booking
    pub owner: Option<String>,
    /// The lab a booking is for
    pub lab: Option<String>,
    /// The purpose of a booking
    pub purpose: Option<String>,
    /// Project a booking belongs to
    pub project: Option<String>,
    /// DateTime<Utc> that contains the start of a booking
    pub start: Option<DateTime<Utc>>,
    /// DateTime<Utc> that contains the end of a booking
    pub end: Option<DateTime<Utc>>,
}

#[derive(Serialize, Deserialize, Debug, Clone, JsonSchema, Eq, PartialEq, Default)]
pub struct AggregateConfiguration {
    pub ipmi_username: String,
    pub ipmi_password: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, Eq, PartialEq, Default)]
pub struct Aggregate {
    pub id: FKey<Aggregate>,
    pub deleted: bool,
    pub users: Vec<String>, // the set of users who should have access to this aggregate
    pub vlans: FKey<NetworkAssignmentMap>,
    pub template: FKey<Template>,
    pub metadata: BookingMetadata,
    pub state: LifeCycleState,
    pub configuration: AggregateConfiguration,
    pub lab: FKey<Lab>, // The originating project for this aggregate
}

impl std::fmt::Display for Aggregate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.id.into_id())
    }
}

impl DBTable for Aggregate {
    fn id(&self) -> ID {
        self.id.into_id()
    }

    fn id_mut(&mut self) -> &mut ID {
        self.id.into_id_mut()
    }

    fn table_name() -> &'static str {
        "aggregates"
    }

    fn from_row(row: tokio_postgres::Row) -> Result<ExistingRow<Self>, anyhow::Error> {
        Ok(ExistingRow::from_existing(Self {
            id: row.try_get("id")?,
            deleted: row.try_get("deleted")?,
            users: row.try_get("users")?,
            vlans: row.try_get("vlans")?,
            template: row.try_get("template")?,
            state: serde_json::from_value(row.try_get("lifecycle_state")?)?,
            metadata: serde_json::from_value(row.try_get("metadata")?)?,
            configuration: serde_json::from_value(row.try_get("configuration")?).unwrap_or(
                AggregateConfiguration {
                    ipmi_username: String::new(),
                    ipmi_password: String::new(),
                },
            ),
            lab: row.try_get("lab")?,
        }))
    }

    fn to_rowlike(&self) -> Result<HashMap<&str, Box<dyn ToSqlObject>>, anyhow::Error> {
        let clone = self.clone();
        let c: [(&str, Box<dyn ToSqlObject>); _] = [
            ("id", Box::new(clone.id)),
            ("deleted", Box::new(clone.deleted)),
            ("users", Box::new(clone.users)),
            ("vlans", Box::new(clone.vlans)),
            ("metadata", Box::new(serde_json::to_value(clone.metadata)?)),
            ("template", Box::new(clone.template)),
            ("lifecycle_state", Box::new(clone.state)),
            ("lab", Box::new(clone.lab)),
            (
                "configuration",
                Box::new(serde_json::to_value(clone.configuration)?),
            ),
        ];

        Ok(c.into_iter().collect())
    }
}

impl Aggregate {
    pub async fn instances(
        &self,
        t: &mut EasyTransaction<'_>,
    ) -> Result<Vec<ExistingRow<Instance>>, anyhow::Error> {
        Instance::select()
            .where_field("aggregate")
            .equals(self.id)
            .run(t)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::collection::vec;
    use proptest::option::of;
    use proptest::prelude::*;
    use testing_utils::{block_on_runtime, datetime_utc_strategy, insert_default_model_at};

    impl Arbitrary for BookingMetadata {
        type Strategy = BoxedStrategy<Self>;
        type Parameters = ();

        fn arbitrary_with(_args: Self::Parameters) -> Self::Strategy {
            (
                of("[a-zA-Z]{1,20}"),
                of("[a-zA-Z]{1,20}"),
                of("[a-zA-Z]{1,20}"),
                of("[a-zA-Z]{1,20}"),
                of("[a-zA-Z]{1,20}"),
                of(datetime_utc_strategy()),
                of(datetime_utc_strategy()),
            )
                .prop_map(|(booking_id, owner, lab, purpose, project, start, end)| {
                    BookingMetadata {
                        booking_id,
                        owner,
                        lab,
                        purpose,
                        project,
                        start,
                        end,
                    }
                })
                .boxed()
        }
    }

    impl Aggregate {
        pub async fn insert_default_at(
            id: FKey<Self>,
            t: &mut EasyTransaction<'_>,
        ) -> Result<(), anyhow::Error> {
            let aggregate = Aggregate {
                id,
                ..Default::default()
            };

            insert_default_model_at(aggregate.template, t).await?;
            insert_default_model_at(aggregate.vlans, t).await?;

            NewRow::new(aggregate).insert(t).await?;

            Ok(())
        }
    }

    impl Arbitrary for AggregateConfiguration {
        type Strategy = BoxedStrategy<Self>;
        type Parameters = ();

        fn arbitrary_with(_args: Self::Parameters) -> Self::Strategy {
            ("[a-zA-Z]{1,20}", "[a-zA-Z]{1,20}")
                .prop_map(|(ipmi_username, ipmi_password)| AggregateConfiguration {
                    ipmi_username,
                    ipmi_password,
                })
                .boxed()
        }
    }

    impl Arbitrary for Aggregate {
        type Strategy = BoxedStrategy<Self>;
        type Parameters = ();

        fn arbitrary_with(_args: Self::Parameters) -> Self::Strategy {
            (
                Just(FKey::new_id_dangling()),   // id
                any::<bool>(),                   // deleted
                vec("[a-zA-Z]{1,20}", 0..10),    // users
                Just(FKey::new_id_dangling()),   // vlans
                any::<FKey<Template>>(),         // template
                any::<BookingMetadata>(),        // metadata
                any::<LifeCycleState>(),         // state
                any::<AggregateConfiguration>(), // configuration
                any::<FKey<Lab>>(),              // lab
            )
                .prop_map(
                    |(id, deleted, users, vlans, template, metadata, state, configuration, lab)| {
                        Aggregate {
                            id,
                            deleted,
                            users,
                            vlans,
                            template,
                            metadata,
                            state,
                            configuration,
                            lab,
                        }
                    },
                )
                .boxed()
        }
    }

    proptest! {
        #[test]
        fn test_aggregate_model(aggregate in any::<Aggregate>()) {
            block_on_runtime!({
                let client = new_client().await;
                prop_assert!(client.is_ok(), "DB connection failed: {:?}", client.err());
                let mut client = client.unwrap();

                let transaction = client.easy_transaction().await;
                prop_assert!(transaction.is_ok(), "Transaction creation failed: {:?}", transaction.err());
                let mut transaction = transaction.unwrap();

                let template_instert_result = insert_default_model_at(aggregate.template, &mut transaction).await;
                prop_assert!(template_instert_result.is_ok(), "Failed to prepare test environment: {:?}", template_instert_result.err());

                let vlans_insert_result = insert_default_model_at(aggregate.vlans, &mut transaction).await;
                prop_assert!(vlans_insert_result.is_ok(), "Failed to prepare test environment: {:?}", vlans_insert_result.err());

                let new_row = NewRow::new(aggregate.clone());
                let insert_result = new_row.insert(&mut transaction).await;
                prop_assert!(insert_result.is_ok(), "Insert failed: {:?}", insert_result.err());

                let retrieved_agg_result = Aggregate::select().where_field("id").equals(aggregate.id).run(&mut transaction)
                    .await;
                prop_assert!(retrieved_agg_result.is_ok(), "Retrieval failed: {:?}", retrieved_agg_result.err());

                let first_agg = retrieved_agg_result.unwrap().into_iter().next();
                prop_assert!(first_agg.is_some(), "No host found, empty result");

                let retrieved_agg = first_agg.unwrap().clone().into_inner();
                prop_assert_eq!(retrieved_agg, aggregate);

                Ok(())
            })?
        }
    }
}
