use anyhow::Result;
use chrono::{DateTime, Utc};
use dal::{web::*, *};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::dashboard::{Instance, ProvEvent, StatusSentiment};

#[derive(Serialize, Deserialize, Debug, Clone, Eq, PartialEq)]
pub struct ProvisionLogEvent {
    pub id: FKey<ProvisionLogEvent>,
    pub sentiment: StatusSentiment,
    pub instance: FKey<Instance>,
    pub time: DateTime<Utc>,
    pub prov_status: ProvEvent,
}

impl DBTable for ProvisionLogEvent {
    fn table_name() -> &'static str {
        "provision_log_events"
    }

    fn id(&self) -> ID {
        self.id.into_id()
    }

    fn id_mut(&mut self) -> &mut ID {
        self.id.into_id_mut()
    }

    fn from_row(row: tokio_postgres::Row) -> Result<ExistingRow<Self>, anyhow::Error> {
        Ok(ExistingRow::from_existing(Self {
            sentiment: row
                .try_get("sentiment")
                .unwrap_or(SqlAsJson(StatusSentiment::Unknown))
                .extract(),
            id: row.try_get("id")?,
            instance: row.try_get("instance")?,
            time: row.try_get("time")?,
            prov_status: serde_json::from_value(row.try_get("prov_status")?)?,
        }))
    }

    fn to_rowlike(&self) -> Result<HashMap<&str, Box<dyn ToSqlObject>>, anyhow::Error> {
        let clone = self.clone();
        let c: [(&str, Box<dyn ToSqlObject>); _] = [
            ("id", Box::new(clone.id)),
            ("sentiment", Box::new(SqlAsJson::of(self.sentiment))),
            ("instance", Box::new(clone.instance)),
            ("time", Box::new(clone.time)),
            (
                "prov_status",
                Box::new(serde_json::to_value(clone.prov_status)?),
            ),
        ];

        Ok(c.into_iter().collect())
    }
}

impl ProvisionLogEvent {
    pub async fn all_for_instance(
        t: &mut EasyTransaction<'_>,
        instance: FKey<Instance>,
    ) -> Result<Vec<ExistingRow<ProvisionLogEvent>>, anyhow::Error> {
        let tn = <Self as DBTable>::table_name();
        let q = format!("SELECT * FROM {tn} WHERE instance = $1;");

        t.query(&q, &[&instance])
            .await
            .map(Self::from_rows)
            .anyway()
            .flatten()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use testing_utils::{block_on_runtime, datetime_utc_strategy};

    impl Arbitrary for ProvisionLogEvent {
        type Strategy = BoxedStrategy<Self>;
        type Parameters = ();

        fn arbitrary_with(_args: Self::Parameters) -> Self::Strategy {
            (
                any::<FKey<ProvisionLogEvent>>(), // id
                any::<StatusSentiment>(),         // sentiment
                any::<FKey<Instance>>(),          // instance
                datetime_utc_strategy(),          // time
                any::<ProvEvent>(),               // porv_status
            )
                .prop_map(
                    |(id, sentiment, instance, time, prov_status)| ProvisionLogEvent {
                        id,
                        sentiment,
                        instance,
                        time,
                        prov_status,
                    },
                )
                .boxed()
        }
    }

    proptest! {
        #[test]
        fn test_provision_log_event_model(provision_log in any::<ProvisionLogEvent>()) {
            block_on_runtime!({
                let client = new_client().await;
                prop_assert!(client.is_ok(), "DB connection failed: {:?}", client.err());
                let mut client = client.unwrap();

                let transaction = client.easy_transaction().await;
                prop_assert!(transaction.is_ok(), "Transaction creation failed: {:?}", transaction.err());
                let mut transaction = transaction.unwrap();

                let instance_insert_result = Instance::insert_default_at(provision_log.instance, &mut transaction).await;
                prop_assert!(instance_insert_result.is_ok(), "Failed to prepare test environment: {:?}", instance_insert_result.err());

                let new_row = NewRow::new(provision_log.clone());
                let insert_result = new_row.insert(&mut transaction).await;
                prop_assert!(insert_result.is_ok(), "Insert failed: {:?}", insert_result.err());

                let retrieved_log_event_result = ProvisionLogEvent::select().where_field("id").equals(provision_log.id).run(&mut transaction)
                    .await;
                prop_assert!(retrieved_log_event_result.is_ok(), "Retrieval failed: {:?}", retrieved_log_event_result.err());

                let first_log_event = retrieved_log_event_result.unwrap().into_iter().next();
                prop_assert!(first_log_event.is_some(), "No host found, empty result");

                let retrieved_log_event = first_log_event.unwrap().clone().into_inner();
                prop_assert_eq!(retrieved_log_event, provision_log);

                Ok(())
            })?
        }
    }
}
