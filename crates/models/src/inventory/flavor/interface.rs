use dal::{web::AnyWay, *};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::inventory::{DataValue, Flavor};

#[derive(Serialize, Deserialize, Debug)]
pub struct InterfaceFlavor {
    pub id: FKey<InterfaceFlavor>,

    pub on_flavor: FKey<Flavor>,
    pub name: String, // Interface name
    pub speed: DataValue,
    pub cardtype: CardType,
}

impl DBTable for InterfaceFlavor {
    fn id(&self) -> ID {
        self.id.into_id()
    }

    fn table_name() -> &'static str {
        "interface_flavors"
    }

    fn from_row(
        row: tokio_postgres::Row,
    ) -> Result<ExistingRow<Self>, common::prelude::anyhow::Error> {
        Ok(ExistingRow::from_existing(Self {
            id: row.try_get("id")?,
            on_flavor: row.try_get("on_flavor")?,

            name: row.try_get("name")?,
            speed: DataValue::from_sqlval(row.try_get("speed")?)?,
            cardtype: serde_json::from_value(row.try_get("cardtype")?)?,
        }))
    }

    fn to_rowlike(&self) -> Result<HashMap<&str, Box<dyn ToSqlObject>>, anyhow::Error> {
        let c: [(&str, Box<dyn ToSqlObject>); _] = [
            ("id", Box::new(self.id)),
            ("on_flavor", Box::new(self.on_flavor)),
            ("name", Box::new(self.name.clone())),
            ("speed", self.speed.to_sqlval()?),
            ("cardtype", Box::new(serde_json::to_value(self.cardtype)?)),
        ];

        Ok(c.into_iter().collect())
    }
}

impl InterfaceFlavor {
    pub async fn all_for_flavor(
        transaction: &mut EasyTransaction<'_>,
        flavor: FKey<Flavor>,
    ) -> Result<Vec<ExistingRow<Self>>, anyhow::Error> {
        let tn = Self::table_name();
        let q = format!("SELECT * FROM {tn} WHERE on_flavor = $1;");
        let rows = transaction.query(&q, &[&flavor]).await.anyway()?;
        Self::from_rows(rows)
    }
}

#[derive(Serialize, Deserialize, Debug, Default, Clone, Hash, Copy, JsonSchema)]
pub enum CardType {
    PCIeOnboard,
    PCIeModular,

    #[default]
    Unknown,
}
