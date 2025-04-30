use anyhow::Result;
use common::prelude::chrono::Utc;
use dal::{web::*, *};

use common::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::inventory::Host;

use crate::dashboard::types::ProvEvent;
use crate::EasyLog;

use crate::dashboard::{
    Aggregate, HostConfig, NetworkAssignmentMap, ProvisionLogEvent, StatusSentiment, Template,
};

#[derive(Serialize, Deserialize, Debug, Clone, Eq, PartialEq, Default)]
pub struct Instance {
    pub id: FKey<Instance>, // Instance id which exists when the host is being provisioned
    pub within_template: FKey<Template>,
    pub aggregate: FKey<Aggregate>,
    pub network_data: FKey<NetworkAssignmentMap>,
    pub linked_host: Option<FKey<Host>>,
    pub config: HostConfig, // Host config
    // TODO: This type does not enforce sanitized keys or `serde_json::Value`'s. It would be a good
    // idea to fix this sometime in the future as allowing escape characters and other pesky stuff
    // in here and throwing it directly into a SQL query is a potential security risk.
    pub metadata: HashMap<String, serde_json::Value>,
}

impl std::hash::Hash for Instance {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.id.hash(state);
        self.within_template.hash(state);
    }
}

impl DBTable for Instance {
    fn table_name() -> &'static str {
        "instances"
    }

    fn id(&self) -> ID {
        self.id.into_id()
    }

    fn id_mut(&mut self) -> &mut ID {
        self.id.into_id_mut()
    }

    fn from_row(row: tokio_postgres::Row) -> Result<ExistingRow<Self>, anyhow::Error> {
        Ok(ExistingRow::from_existing(Self {
            id: row.try_get("id")?,
            within_template: row.try_get("within_template")?,
            aggregate: row.try_get("aggregate")?,
            network_data: row.try_get("network_data")?,
            linked_host: row.try_get("linked_host")?,
            config: serde_json::from_value(row.try_get("config")?)?,
            metadata: serde_json::from_value(row.try_get("metadata")?)?,
        }))
    }

    fn to_rowlike(&self) -> Result<HashMap<&str, Box<dyn ToSqlObject>>, anyhow::Error> {
        let clone = self.clone();
        let c: [(&str, Box<dyn ToSqlObject>); _] = [
            ("id", Box::new(clone.id)),
            ("within_template", Box::new(clone.within_template)),
            ("aggregate", Box::new(clone.aggregate)),
            ("network_data", Box::new(clone.network_data)),
            ("linked_host", Box::new(clone.linked_host)),
            ("config", Box::new(serde_json::to_value(clone.config)?)),
            ("metadata", Box::new(serde_json::to_value(clone.metadata)?)),
        ];

        Ok(c.into_iter().collect())
    }
}

impl Instance {
    pub async fn log(
        inst: FKey<Instance>,
        transaction: &mut EasyTransaction<'_>,
        event: ProvEvent,
        sentiment: Option<StatusSentiment>,
    ) -> Result<(), anyhow::Error> {
        let ple = ProvisionLogEvent {
            id: FKey::new_id_dangling(),
            sentiment: sentiment.unwrap_or(StatusSentiment::Unknown),
            instance: inst,
            time: Utc::now(),
            prov_status: event,
        };

        let nr = NewRow::new(ple);

        nr.insert(transaction).await?;

        Ok(())
    }

    pub async fn log_committing(
        inst: FKey<Instance>,
        event: ProvEvent,
        sentiment: Option<StatusSentiment>,
    ) -> Result<(), anyhow::Error> {
        let mut client = new_client().await.log_db_client_error().unwrap();
        let mut transaction = client
            .easy_transaction()
            .await
            .log_db_client_error()
            .unwrap();

        Instance::log(inst, &mut transaction, event, sentiment).await?;
        transaction.commit().await?;

        Ok(())
    }
}

impl EasyLog for FKey<Instance> {
    async fn log<H, D>(&self, header: H, detail: D, status: StatusSentiment)
    where
        H: Into<String>,
        D: Into<String>,
    {
        let header: String = header.into();
        let detail: String = detail.into();

        tracing::info!("Dispatching log for an instance, header: {header}, detail: {detail}");
        let _ = Instance::log_committing(
            *self,
            ProvEvent {
                event: header,
                details: detail,
            },
            Some(status),
        )
        .await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::option::of;
    use proptest::prelude::*;
    use testing_utils::{arb_json_map, block_on_runtime, insert_default_model_at};

    impl Arbitrary for Instance {
        type Parameters = ();
        type Strategy = BoxedStrategy<Self>;

        fn arbitrary_with(_args: ()) -> Self::Strategy {
            (
                any::<FKey<Instance>>(),             // id
                any::<FKey<Template>>(),             // within_template
                any::<FKey<Aggregate>>(),            // aggregate
                any::<FKey<NetworkAssignmentMap>>(), // network_data
                of(any::<FKey<Host>>()),             // linked_host
                any::<HostConfig>(),                 // config
                arb_json_map::<String>(0..10),       // metadata
            )
                .prop_map(
                    |(
                        id,
                        within_template,
                        aggregate,
                        network_data,
                        linked_host,
                        config,
                        metadata,
                    )| {
                        Instance {
                            id,
                            within_template,
                            aggregate,
                            network_data,
                            linked_host,
                            config,
                            metadata,
                        }
                    },
                )
                .boxed()
        }
    }

    impl Instance {
        pub async fn insert_default_at(
            id: FKey<Instance>,
            t: &mut EasyTransaction<'_>,
        ) -> Result<()> {
            let instance = Instance {
                id,
                ..Default::default()
            };

            // template
            insert_default_model_at(instance.within_template, t).await?;

            // aggregate
            Aggregate::insert_default_at(instance.aggregate, t).await?;

            // network assignment map
            insert_default_model_at(instance.network_data, t).await?;

            // optional linked host
            if let Some(linked_host) = instance.linked_host {
                insert_default_model_at(linked_host, t).await?;
            }

            // instance (self)
            SchrodingerRow::new(instance).upsert(t).await?;

            Ok(())
        }
    }

    proptest! {
        #[test]
        fn test_instance_model(instance in any::<Instance>()) {
            block_on_runtime!({
                let client = new_client().await;
                prop_assert!(client.is_ok(), "DB connection failed: {:?}", client.err());
                let mut client = client.unwrap();
                let transaction = client.easy_transaction().await;
                prop_assert!(transaction.is_ok(), "Transaction creation failed: {:?}", transaction.err());
                let mut transaction = transaction.unwrap();

                let template_insert_result = insert_default_model_at(instance.within_template, &mut transaction).await;
                prop_assert!(template_insert_result.is_ok(), "Insert failed while trying to prepare test: {:?}", template_insert_result.err());

                let aggregate_insert_result = Aggregate::insert_default_at(instance.aggregate, &mut transaction).await;
                prop_assert!(aggregate_insert_result.is_ok(), "Insert failed while trying to prepare test: {:?}", aggregate_insert_result.err());

                let network_map_insert_result = insert_default_model_at(instance.network_data, &mut transaction).await;
                prop_assert!(network_map_insert_result.is_ok(), "Insert failed while trying to prepare test: {:?}", network_map_insert_result.err());

                if let Some(host_fkey) = instance.linked_host {
                    let host_insert_result = Host::insert_default_at(host_fkey, &mut transaction).await;
                    prop_assert!(host_insert_result.is_ok(), "Insert failed while trying to prepare test: {:?}", host_insert_result.err());
                }

                let new_row = NewRow::new(instance.clone());
                let insert_result = new_row.insert(&mut transaction).await;
                prop_assert!(insert_result.is_ok(), "Insert failed: {:?}", insert_result.err());

                let retrieved_instance_result = Instance::select()
                    .where_field("id")
                    .equals(instance.id)
                    .run(&mut transaction)
                    .await;

                prop_assert!(retrieved_instance_result.is_ok(), "Retrieval failed: {:?}", retrieved_instance_result.err());
                let retrieved_instances = retrieved_instance_result.unwrap();

                let instance_row = retrieved_instances.first();
                prop_assert!(instance_row.is_some(), "No extra flavor info found, empty result");

                let retrieved_instance = instance_row.unwrap().clone().into_inner();

                prop_assert_eq!(retrieved_instance, instance);

                Ok(())
            })?
        }
    }
}
