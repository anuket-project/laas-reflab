use common::prelude::*;
use dal::*;
use serde::{Deserialize, Serialize};
use serde_json::{from_value, to_value};
use std::collections::HashMap;

use mac_address::MacAddress;
use sqlx::FromRow;
use sqlx::{query_as, PgPool};

use crate::inventory::{DataValue, Host, SwitchPort};

#[derive(Serialize, Deserialize, Debug, Clone, Eq, PartialEq, FromRow, sqlx::Type)]
pub struct HostPort {
    pub id: FKey<HostPort>,
    pub on_host: FKey<Host>,
    pub switchport: Option<FKey<SwitchPort>>,
    pub name: String,
    pub speed: DataValue,
    pub mac: MacAddress,
    pub switch: String,
    pub bus_addr: String,
    pub bmc_vlan_id: Option<i16>,
    pub management_vlan_id: Option<i16>,
}

impl DBTable for HostPort {
    fn id(&self) -> ID {
        self.id.into_id()
    }

    fn id_mut(&mut self) -> &mut ID {
        self.id.into_id_mut()
    }

    fn table_name() -> &'static str {
        "host_ports"
    }

    fn from_row(row: tokio_postgres::Row) -> Result<ExistingRow<Self>, anyhow::Error> {
        let speed: DataValue = from_value(row.try_get("speed")?)?;

        // TODO: This is dumb but necessary for now because `tokio_postgres` doesn't implement
        // to_sql for `mac_address::MacAddress` and we can't use `eui48::MacAddress` with sqlx
        // once we fully deprecate `DBTable` this can be removed
        let temp_mac: eui48::MacAddress = row.try_get("mac")?;
        let mac = MacAddress::new(temp_mac.to_array());

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
            management_vlan_id: row.try_get("management_vlan_id")?,
        }))
    }

    fn to_rowlike(&self) -> Result<HashMap<&str, Box<dyn ToSqlObject>>, anyhow::Error> {
        let clone = self.clone();

        let speed = to_value(clone.speed)?;

        // TODO: This is dumb but necessary for now because `tokio_postgres` doesn't implement
        // to_sql for `mac_address::MacAddress` and we can't use `eui48::MacAddress` with sqlx
        // once we fully deprecate `DBTable` this can be removed
        let mac = eui48::MacAddress::from_bytes(&clone.mac.bytes()).map_err(|e| {
            anyhow::anyhow!(
                "Failed to convert mac_address::MacAddress to eui48::MacAddress as bytes: {}",
                e
            )
        })?;

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
            ("management_vlan_id", Box::new(clone.management_vlan_id)),
        ];

        Ok(c.into_iter().collect())
    }
}

impl HostPort {
    /// Fetch all ports for a given host ID.
    pub async fn all_for_host(pool: &PgPool, host_id: FKey<Host>) -> anyhow::Result<Vec<Self>> {
        let sql = r#"
            SELECT id, on_host, switchport, name,
                   speed, mac, switch, bus_addr,
                   bmc_vlan_id, management_vlan_id
              FROM host_ports
             WHERE on_host = $1
        "#;

        let ports = query_as::<_, HostPort>(sql)
            .bind(host_id)
            .fetch_all(pool)
            .await?;

        Ok(ports)
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use prop::option::of;
    use proptest::prelude::*;
    use testing_utils::{block_on_runtime, mac_address_strategy};

    pub fn host_port_strategy() -> impl Strategy<Value = HostPort> {
        (
            any::<FKey<HostPort>>(),       // id
            any::<FKey<Host>>(),           // on_host
            of(any::<FKey<SwitchPort>>()), // switchport
            "[a-zA-Z0-9-]{1,50}",          // name
            any::<DataValue>(),            // speed
            mac_address_strategy(),        // mac
            "[a-zA-Z0-9]{1,20}",           // switch
            "[a-zA-Z0-9]{1,20}",           // bus_addr
            of(i16::MIN..i16::MAX),        // bmc_vlan_id
            of(i16::MIN..i16::MAX),        // management_vlan_id
        )
            .prop_map(
                |(
                    id,
                    on_host,
                    switchport,
                    name,
                    speed,
                    mac,
                    switch,
                    bus_addr,
                    bmc_vlan_id,
                    management_vlan_id,
                )| HostPort {
                    id,
                    on_host,
                    switchport,
                    name: name.to_string(),
                    speed,
                    mac,
                    switch: switch.to_string(),
                    bus_addr: bus_addr.to_string(),
                    bmc_vlan_id,
                    management_vlan_id,
                },
            )
    }

    proptest! {
        #[test]
        fn test_host_port_model(host_port in host_port_strategy()) {
            block_on_runtime!({
                let client = new_client().await;
                prop_assert!(client.is_ok(), "DB connection failed: {:?}", client.err());
                let mut client = client.unwrap();

                let transaction = client.easy_transaction().await;
                prop_assert!(transaction.is_ok(), "Transaction creation failed: {:?}", transaction.err());
                let mut transaction = transaction.unwrap();

                let host_insert_result = Host::insert_default_at(host_port.on_host, &mut transaction).await;
                prop_assert!(host_insert_result.is_ok(), "Failed to prepare test environment: {:?}", host_insert_result.err());

                let new_row = NewRow::new(host_port.clone());
                let insert_result = new_row.insert(&mut transaction).await;
                prop_assert!(insert_result.is_ok(), "Insert failed: {:?}", insert_result.err());

                let retrieved_host_port = HostPort::select()
                    .where_field("id")
                    .equals(host_port.id)
                    .run(&mut transaction)
                    .await;

                prop_assert!(retrieved_host_port.is_ok(), "Retrieval failed: {:?}", retrieved_host_port.err());
                let retrieved_host_port = retrieved_host_port.unwrap();

                let first_host_port = retrieved_host_port.first();
                prop_assert!(first_host_port.is_some(), "No host port found");

                let retrieved_host_port = first_host_port.unwrap().clone().into_inner();

                prop_assert_eq!(retrieved_host_port, host_port);

                Ok(())
            })?
        }
    }
}
