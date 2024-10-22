use dal::{web::*, *};
use tokio_postgres::types::ToSql;

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use common::prelude::*;

// TODO: Delete this bc it should not exist
#[derive(Serialize, Deserialize, Debug, Clone, Hash)]
pub struct VPNToken {
    pub id: FKey<VPNToken>,
    pub username: String,
    pub project: String,
}

impl DBTable for VPNToken {
    fn table_name() -> &'static str {
        "vpn_tokens"
    }

    fn id(&self) -> ID {
        self.id.into_id()
    }

    fn from_row(row: tokio_postgres::Row) -> Result<ExistingRow<Self>, anyhow::Error> {
        let id = row.try_get("id").anyway()?;
        let username = row.try_get("username").anyway()?;
        let project = row.try_get("project").anyway()?;

        Ok(ExistingRow::from_existing(Self {
            id,
            username,
            project,
        }))
    }

    fn to_rowlike(&self) -> Result<HashMap<&str, Box<dyn ToSql + Sync + Send>>, anyhow::Error> {
        let Self {
            id,
            username,
            project,
        } = self.clone();
        let c: [(&str, Box<dyn tokio_postgres::types::ToSql + Sync + Send>); _] = [
            ("id", Box::new(id)),
            ("username", Box::new(username)),
            ("project", Box::new(project)),
        ];

        Ok(c.into_iter().collect())
    }
}
