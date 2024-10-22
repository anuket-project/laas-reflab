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

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Instance {
    pub id: FKey<Instance>, // Instance id which exists when the host is being provisioned

    pub within_template: FKey<Template>,

    pub aggregate: FKey<Aggregate>,

    pub network_data: FKey<NetworkAssignmentMap>,

    pub linked_host: Option<FKey<Host>>,

    pub config: HostConfig, // Host config

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
