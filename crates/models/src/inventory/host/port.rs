use common::prelude::{itertools::Itertools, macaddr::MacAddr6, *};
use dal::{web::AnyWay, *};
use serde::{Deserialize, Serialize};
use serde_json::{from_value, to_value};
use std::collections::HashMap;

use crate::inventory::{DataValue, Host, InterfaceFlavor, SwitchPort};

#[derive(Serialize, Deserialize, Debug, Clone, Eq, PartialEq)]
pub struct HostPort {
    pub id: FKey<HostPort>,

    pub on_host: FKey<Host>,
    pub switchport: Option<FKey<SwitchPort>>,
    pub name: String,
    pub speed: DataValue,
    pub mac: MacAddr6,
    pub switch: String,
    pub bus_addr: String,
    pub bmc_vlan_id: Option<i16>,

    pub is_a: FKey<InterfaceFlavor>,
}

impl DBTable for HostPort {
    fn id(&self) -> ID {
        self.id.into_id()
    }

    fn table_name() -> &'static str {
        "host_ports"
    }

    fn from_row(row: tokio_postgres::Row) -> Result<ExistingRow<Self>, anyhow::Error> {
        let speed: DataValue = from_value(row.try_get("speed")?)?;
        let mac = from_value(row.try_get("mac")?)?;

        Ok(ExistingRow::from_existing(Self {
            id: row.try_get("id")?,
            on_host: row.try_get("on_host")?,
            switchport: row.try_get("switchport")?,
            name: row.try_get("name")?,
            speed,
            mac,
            switch: row.try_get("switch")?,
            bus_addr: row.try_get("bus_addr")?,
            bmc_vlan_id: row.try_get("bmc_vlan_id")?,
            is_a: row.try_get("is_a")?,
        }))
    }

    fn to_rowlike(&self) -> Result<HashMap<&str, Box<dyn ToSqlObject>>, anyhow::Error> {
        let clone = self.clone();

        let speed = to_value(clone.speed)?;
        let mac = to_value(clone.mac)?;

        let c: [(&str, Box<dyn ToSqlObject>); _] = [
            ("id", Box::new(self.id)),
            ("on_host", Box::new(self.on_host)),
            ("switchport", Box::new(self.switchport)),
            ("name", Box::new(clone.name)),
            ("speed", Box::new(speed)),
            ("mac", Box::new(mac)),
            ("switch", Box::new(clone.switch)),
            ("bus_addr", Box::new(clone.bus_addr)),
            ("bmc_vlan_id", Box::new(clone.bmc_vlan_id)),
            ("is_a", Box::new(self.is_a)),
        ];

        Ok(c.into_iter().collect())
    }
}

impl HostPort {
    pub async fn all_for_host(
        t: &mut EasyTransaction<'_>,
        pk: FKey<Host>,
    ) -> Result<Vec<HostPort>, anyhow::Error> {
        let tn = <Self as DBTable>::table_name();
        let q = format!("SELECT * FROM {tn} WHERE on_host = $1;");

        let rows = t.query(&q, &[&pk]).await.anyway()?;

        Ok(Self::from_rows(rows)?
            .into_iter()
            .map(|er| er.into_inner())
            .collect_vec())
    }
}
