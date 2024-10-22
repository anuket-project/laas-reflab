use dal::{web::*, *};
use tokio_postgres::types::ToSql;

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::{allocator::ResourceHandle, dashboard::Aggregate, inventory::*};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Allocation {
    pub id: FKey<Allocation>,
    pub for_resource: FKey<ResourceHandle>,
    pub for_aggregate: Option<FKey<Aggregate>>,

    pub started: chrono::DateTime<chrono::Utc>,

    pub ended: Option<chrono::DateTime<chrono::Utc>>,

    pub reason_started: AllocationReason,

    pub reason_ended: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy)]
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

    fn to_rowlike(&self) -> Result<HashMap<&str, Box<dyn ToSql + Sync + Send>>, anyhow::Error> {
        let c: [(&str, Box<dyn tokio_postgres::types::ToSql + Sync + Send>); _] = [
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

    pub async fn all_for_aggregate(
        t: &mut EasyTransaction<'_>,
        agg: FKey<Aggregate>,
    ) -> Result<Vec<ExistingRow<Allocation>>, anyhow::Error> {
        let tn = Self::table_name();

        let q = format!("SELECT * FROM {tn} WHERE for_aggregate = $1;");

        let rows = t.query(&q, &[&Some(agg)]).await.anyway()?;
        Self::from_rows(rows)
    }

    /// selects for the given aggregate and host
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
            true => "is null",
            false => "is not null",
        };

        let q = format!("select * from {allocation_tn} where for_aggregate = $1 and ended {is_null} and for_resource = ({rh_id})");

        let rows = t.query(&q, &[&agg, &host]).await.anyway()?;

        Allocation::from_rows(rows)
    }
}
