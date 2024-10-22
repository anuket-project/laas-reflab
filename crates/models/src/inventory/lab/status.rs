use crate::inventory::Lab;
use chrono::{DateTime, Utc};
use dal::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum LabStatusInner {
    Operational(),
    Recovered(),
    Monitoring(),
    StatusUpdate(),
    UnplannedMaintenance(),
    PlannedMaintenance(),
    Degraded(),
    Decommissioned(),
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct LabStatusUpdate {
    pub headline: String,
    pub elaboration: Option<String>,
    pub time: DateTime<Utc>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Forecast {
    pub expected_time: Option<DateTime<Utc>>,
    pub explanation: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct LabStatus {
    pub id: FKey<LabStatus>,
    pub for_lab: FKey<Lab>,

    pub time: DateTime<Utc>,

    pub expected_next_event_time: Forecast,
    pub status: LabStatusInner,
    pub headline: Option<String>,
    pub subline: Option<String>,
}

impl DBTable for LabStatus {
    fn id(&self) -> ID {
        self.id.into_id()
    }

    fn table_name() -> &'static str {
        "lab_statuses"
    }

    fn from_row(row: tokio_postgres::Row) -> Result<ExistingRow<Self>, anyhow::Error> {
        Ok(ExistingRow::from_existing(Self {
            id: row.try_get("id")?,
            for_lab: row.try_get("for_lab")?,
            time: row.try_get("time")?,
            expected_next_event_time: serde_json::from_value(
                row.try_get("expected_next_event_time")?,
            )?,
            status: serde_json::from_value(row.try_get("status")?)?,
            headline: row.try_get("headline")?,
            subline: row.try_get("subline")?,
        }))
    }

    fn to_rowlike(&self) -> Result<HashMap<&str, Box<dyn ToSqlObject>>, anyhow::Error> {
        let c: [(&str, Box<dyn ToSqlObject>); _] = [
            ("id", Box::new(self.id)),
            ("for_lab", Box::new(self.for_lab)),
            ("time", Box::new(self.time)),
            (
                "expected_next_event_time",
                Box::new(serde_json::to_value(self.expected_next_event_time.clone())?),
            ),
            (
                "status",
                Box::new(serde_json::to_value(self.status.clone())?),
            ),
            ("headline", Box::new(self.headline.clone())),
            ("subline", Box::new(self.subline.clone())),
        ];

        Ok(c.into_iter().collect())
    }
}
