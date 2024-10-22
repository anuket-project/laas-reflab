use anyhow::Result;
use chrono::{DateTime, Utc};
use dal::{web::*, *};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::dashboard::{Instance, ProvEvent, StatusSentiment};

#[derive(Serialize, Deserialize, Debug, Clone)]
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
