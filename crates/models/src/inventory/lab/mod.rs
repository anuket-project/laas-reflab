use serde::{Deserialize, Serialize};
use std::{cmp::Ordering, collections::HashMap};

use dal::{web::*, *};

mod status;

pub use status::LabStatus;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Lab {
    pub id: FKey<Lab>,
    pub name: String,
    pub location: String,
    pub email: String,
    pub phone: String,
    pub is_dynamic: bool,
}

impl Lab {
    pub async fn status(&self) -> Option<FKey<LabStatus>> {
        let mut client = new_client().await.expect("Expected to connect to db");
        let mut transaction = client
            .easy_transaction()
            .await
            .expect("Transaction creation error");
        let stati = LabStatus::select()
            .where_field("for_lab")
            .equals(self.id)
            .run(&mut transaction)
            .await
            .expect("Statuses for lab not found");
        match stati.len() {
            0 | 1 => stati.first().map(|s| s.id),
            _ => {
                let mut largest: ExistingRow<LabStatus> = stati
                    .first()
                    .expect("Expected to have a lab status")
                    .clone();
                for status in stati {
                    if largest.time.cmp(&status.time) == Ordering::Less {
                        largest = status;
                    }
                }
                Some(largest.id)
            }
        }
    }

    pub async fn get_by_name(
        transaction: &mut EasyTransaction<'_>,
        name: String,
    ) -> Result<Option<ExistingRow<Lab>>, anyhow::Error> {
        let tn = <Self as DBTable>::table_name();
        let q = format!("SELECT * FROM {tn} WHERE name = '{name}';");
        println!("{q}");

        let opt_row = transaction.query_opt(&q, &[]).await.anyway()?;
        Ok(match opt_row {
            None => None,
            Some(row) => Some(Self::from_row(row)?),
        })
    }
}

impl DBTable for Lab {
    fn id(&self) -> ID {
        self.id.into_id()
    }

    fn table_name() -> &'static str {
        "labs"
    }

    fn from_row(row: tokio_postgres::Row) -> Result<ExistingRow<Self>, anyhow::Error> {
        Ok(ExistingRow::from_existing(Self {
            id: row.try_get("id")?,
            name: row.try_get("name")?,
            location: row.try_get("location")?,
            email: row.try_get("email")?,
            phone: row.try_get("phone")?,
            is_dynamic: row.try_get("is_dynamic")?,
        }))
    }

    fn to_rowlike(&self) -> Result<HashMap<&str, Box<dyn ToSqlObject>>, anyhow::Error> {
        let c: [(&str, Box<dyn ToSqlObject>); _] = [
            ("id", Box::new(self.id)),
            ("name", Box::new(self.name.clone())),
            ("location", Box::new(self.location.clone())),
            ("email", Box::new(self.email.clone())),
            ("phone", Box::new(self.phone.clone())),
            ("is_dynamic", Box::new(self.is_dynamic)),
        ];

        Ok(c.into_iter().collect())
    }
}
