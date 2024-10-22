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

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
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

#[derive(Serialize, Deserialize, Debug, Clone, JsonSchema)]
pub struct AggregateConfiguration {
    pub ipmi_username: String,
    pub ipmi_password: String,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Aggregate {
    pub id: FKey<Aggregate>,

    pub deleted: bool,

    pub users: Vec<String>, // the set of users who should have access to this aggregate

    pub vlans: FKey<NetworkAssignmentMap>,

    pub template: FKey<Template>,

    pub metadata: BookingMetadata,

    pub state: LifeCycleState,

    pub configuration: AggregateConfiguration,

    /// The originating project for this aggregate
    pub lab: FKey<Lab>,
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
