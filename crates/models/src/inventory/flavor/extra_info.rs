use dal::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::inventory::Flavor;

#[derive(Serialize, Deserialize, Debug)]
pub struct ExtraFlavorInfo {
    pub id: FKey<ExtraFlavorInfo>,

    pub for_flavor: FKey<Flavor>,
    // Format from flavors doc: "trait:<trait_name> = value". Can be used to require or forbid hardware with the 'required' and 'forbidden' values.
    pub extra_trait: String, // Trait the key value pair appies to (e.g. 'quota', 'hw', 'hw_rng', 'pci_passthrough', 'os')
    pub key: String,
    pub value: String,
}

impl DBTable for ExtraFlavorInfo {
    fn id(&self) -> ID {
        self.id.into_id()
    }

    fn table_name() -> &'static str {
        "extra_flavor_info"
    }

    fn from_row(
        row: tokio_postgres::Row,
    ) -> Result<ExistingRow<ExtraFlavorInfo>, common::prelude::anyhow::Error> {
        Ok(ExistingRow::from_existing(Self {
            id: row.try_get("id")?,
            for_flavor: row.try_get("for_flavor")?,

            extra_trait: row.try_get("extra_trait")?,
            key: row.try_get("key")?,
            value: row.try_get("value")?,
        }))
    }

    fn to_rowlike(&self) -> Result<HashMap<&str, Box<dyn ToSqlObject>>, anyhow::Error> {
        let c: [(&str, Box<dyn ToSqlObject>); _] = [
            col("id", self.id),
            col("for_flavor", self.for_flavor),
            col("extra_trait", self.extra_trait.clone()),
            col("key", self.key.clone()),
            col("value", self.value.clone()),
        ];

        Ok(c.into_iter().collect())
    }
}
