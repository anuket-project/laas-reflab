use dal::{web::AnyWay, *};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tokio_postgres::Row;

use crate::inventory::Host;

#[derive(Serialize, Deserialize, Debug, Clone, Eq, PartialEq, Hash)]
pub struct Action {
    id: FKey<Action>,
    for_host: FKey<Host>,

    /// The tascii action that this action tracks
    in_tascii: ID,

    is_complete: bool,
}

impl DBTable for Action {
    fn id(&self) -> ID {
        self.id.into_id()
    }

    fn table_name() -> &'static str {
        "host_actions"
    }
    // JSONMODEL -> DBTABLE
    fn from_row(row: Row) -> Result<ExistingRow<Self>, anyhow::Error> {
        Ok(ExistingRow::from_existing(Self {
            id: row.try_get("id")?,
            for_host: row.try_get("for_host")?,
            in_tascii: row.try_get("in_tascii")?,
            is_complete: row.try_get("is_complete")?,
        }))
    }

    fn to_rowlike(&self) -> Result<HashMap<&str, Box<dyn ToSqlObject>>, anyhow::Error> {
        let clone = self.clone();
        let c: [(&str, Box<dyn ToSqlObject>); _] = [
            ("id", Box::new(clone.id)),
            ("for_host", Box::new(clone.for_host)),
            ("in_tascii", Box::new(clone.in_tascii)),
            ("is_complete", Box::new(clone.is_complete)),
        ];

        Ok(c.into_iter().collect())
    }
}

impl Action {
    pub async fn get_all_incomplete_for_host(
        t: &mut EasyTransaction<'_>,
        host: FKey<Host>,
    ) -> Result<Vec<ExistingRow<Action>>, anyhow::Error> {
        let tn = <Self as DBTable>::table_name();
        let q = format!("SELECT * FROM {tn} WHERE is_complete = $1 AND for_host = $2;");

        let res = t.query(&q, &[&false, &host]).await.anyway()?;

        Self::from_rows(res)
    }

    pub async fn add_for_host(
        t: &mut EasyTransaction<'_>,
        host: FKey<Host>,
        is_complete: bool,
        in_tascii: ID,
    ) -> Result<FKey<Action>, anyhow::Error> {
        let action = NewRow::new(Action {
            id: FKey::new_id_dangling(),
            for_host: host,
            is_complete,
            in_tascii,
        });

        action.insert(t).await
    }
}
