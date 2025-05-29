use dal::{web::AnyWay, *};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

mod port;
pub use port::HostPort;

use crate::inventory::Flavor;

#[derive(Serialize, Deserialize, Clone, Debug, Eq, PartialEq, Default)]
pub struct Host {
    pub id: FKey<Host>,
    pub server_name: String,
    pub flavor: FKey<Flavor>, // Flavor used during provisioning
    pub serial: String,
    pub ipmi_fqdn: String,
    pub iol_id: String,
    pub ipmi_mac: eui48::MacAddress,
    pub ipmi_user: String,
    pub ipmi_pass: String,
    pub fqdn: String,
    pub projects: Vec<String>,
    pub sda_uefi_device: Option<String>,
}

impl Named for Host {
    fn name_parts(&self) -> Vec<String> {
        vec![self.server_name.clone()]
    }

    fn name_columnnames() -> Vec<String> {
        vec!["server_name".to_owned()]
    }
}

impl Lookup for Host {}

impl DBTable for Host {
    fn id(&self) -> ID {
        self.id.into_id()
    }

    fn id_mut(&mut self) -> &mut ID {
        self.id.into_id_mut()
    }

    fn table_name() -> &'static str {
        "hosts"
    }

    fn from_row(row: tokio_postgres::Row) -> Result<ExistingRow<Self>, anyhow::Error> {
        Ok(ExistingRow::from_existing(Self {
            id: row.try_get("id")?,
            server_name: row.try_get("server_name")?,
            flavor: row.try_get("flavor")?,
            serial: row.try_get("serial")?,
            ipmi_fqdn: row.try_get("ipmi_fqdn")?,
            iol_id: row.try_get("iol_id")?,
            ipmi_mac: row.try_get("ipmi_mac")?,
            ipmi_user: row.try_get("ipmi_user")?,
            ipmi_pass: row.try_get("ipmi_pass")?,
            fqdn: row.try_get("fqdn")?,
            projects: serde_json::from_value(row.try_get("projects")?)?,
            sda_uefi_device: row.try_get("sda_uefi_device")?,
        }))
    }

    fn to_rowlike(&self) -> Result<HashMap<&str, Box<dyn ToSqlObject>>, anyhow::Error> {
        let clone = self.clone();
        let c: [(&str, Box<dyn ToSqlObject>); _] = [
            ("id", Box::new(self.id)),
            ("server_name", Box::new(self.server_name.clone())),
            ("iol_id", Box::new(self.iol_id.clone())),
            ("flavor", Box::new(clone.flavor)),
            ("serial", Box::new(clone.serial)),
            ("ipmi_fqdn", Box::new(clone.ipmi_fqdn)),
            ("ipmi_mac", Box::new(clone.ipmi_mac)),
            ("ipmi_user", Box::new(clone.ipmi_user)),
            ("ipmi_pass", Box::new(clone.ipmi_pass)),
            ("fqdn", Box::new(clone.fqdn)),
            ("projects", Box::new(serde_json::to_value(clone.projects)?)),
            ("sda_uefi_device", Box::new(clone.sda_uefi_device)),
        ];

        Ok(c.into_iter().collect())
    }
}

impl Host {
    pub async fn ports(
        &self,
        _t: &mut EasyTransaction<'_>,
    ) -> Result<Vec<HostPort>, anyhow::Error> {
        let pool = dal::get_db_pool().await?;
        let r = HostPort::all_for_host(&pool, self.id).await;

        tracing::info!("Ports for host {:?} are {:?}", self.id, r);

        r
    }

    pub async fn all_hosts(
        client: &mut EasyTransaction<'_>,
    ) -> Result<Vec<ExistingRow<Host>>, anyhow::Error> {
        let q = format!("SELECT * FROM {};", Self::table_name());
        let rows = client.query(&q, &[]).await.anyway()?;

        // TODO: make it so that we log map failures for rows here, that is useful debug info!
        Ok(rows
            .into_iter()
            .filter_map(|row| Host::from_row(row).ok())
            .collect())
    }

    pub async fn get_by_name(
        client: &mut EasyTransaction<'_>,
        name: String,
    ) -> Result<ExistingRow<Host>, anyhow::Error> {
        let tn = <Self as DBTable>::table_name();
        let q = format!("SELECT * FROM {tn} WHERE server_name = $1;");
        let r = client.query_opt(&q, &[&name]).await.anyway()?;
        let row = r.ok_or(anyhow::Error::msg(format!(
            "No host existed by name {name}"
        )))?;

        let host = Self::from_row(row)?;

        Ok(host)
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use proptest::prelude::*;
    use testing_utils::{block_on_runtime, insert_default_model_at, mac_address_strategy_eui48};

    fn projects_strategy() -> impl Strategy<Value = Vec<String>> {
        proptest::collection::vec("[a-zA-Z0-9]{1, 20}", 0..5)
    }

    fn sda_uefi_device_strategy() -> impl Strategy<Value = Option<String>> {
        proptest::option::of("[a-zA-Z0-9]{1, 20}")
    }

    pub fn host_strategy() -> impl Strategy<Value = Host> {
        (
            // `Arbitray` is only implemented for tuples with up to 12 elements, so we have to
            // split it here since we have 13
            (
                any::<FKey<Host>>(),   // id
                "[a-zA-Z0-9-]{1,50}",  // server_name
                any::<FKey<Flavor>>(), // flavor
                "[a-zA-Z0-9]{1,20}",   // serial
                "[a-zA-Z0-9.-]{1,50}", // ipmi_fqdn
                "[a-zA-Z0-9]{1,20}",   // iol_id
            ),
            (
                mac_address_strategy_eui48(), // iol_mac
                "[a-zA-Z0-9]{1,20}",          // ipmi_user
                "[a-zA-Z0-9]{1,20}",          // ipmi_pass
                "[a-zA-Z0-9.-]{1,50}",        // fqdn
                projects_strategy(),          // projects
                sda_uefi_device_strategy(),   // sda_uefi_device
            ),
        )
            .prop_map(
                |(
                    (id, server_name, flavor, serial, ipmi_fqdn, iol_id),
                    (ipmi_mac, ipmi_user, ipmi_pass, fqdn, projects, sda_uefi_device),
                )| Host {
                    id,
                    server_name,
                    flavor,
                    serial,
                    ipmi_fqdn,
                    iol_id,
                    ipmi_mac,
                    ipmi_user,
                    ipmi_pass,
                    fqdn,
                    projects,
                    sda_uefi_device,
                },
            )
    }

    impl Host {
        pub async fn insert_default_at(
            id: FKey<Host>,
            transaction: &mut EasyTransaction<'_>,
        ) -> Result<(), anyhow::Error> {
            let host = Host {
                id,
                ..Default::default()
            };

            insert_default_model_at(host.flavor, transaction).await?;

            SchrodingerRow::new(host).upsert(transaction).await?;

            Ok(())
        }
    }

    proptest! {
        #[test]
        fn test_host_model(host in host_strategy()) {
            block_on_runtime!({
                let client = new_client().await;
                prop_assert!(client.is_ok(), "DB connection failed: {:?}", client.err());
                let mut client = client.unwrap();

                let transaction = client.easy_transaction().await;
                prop_assert!(transaction.is_ok(), "Transaction creation failed: {:?}", transaction.err());
                let mut transaction = transaction.unwrap();

                let flavor_insert_result = insert_default_model_at(host.flavor, &mut transaction).await;
                prop_assert!(flavor_insert_result.is_ok(), "Insert failed: {:?}", flavor_insert_result.err());

                let new_row = NewRow::new(host.clone());
                let insert_result = new_row.insert(&mut transaction).await;
                prop_assert!(insert_result.is_ok(), "Insert failed: {:?}", insert_result.err());

                let retrieved_host_result = Host::select().where_field("id").equals(host.id).run(&mut transaction)
                    .await;
                prop_assert!(retrieved_host_result.is_ok(), "Retrieval failed: {:?}", retrieved_host_result.err());

                let first_host = retrieved_host_result.unwrap().into_iter().next();
                prop_assert!(first_host.is_some(), "No host found, empty result");

                let retrieved_host = first_host.unwrap().clone().into_inner();
                prop_assert_eq!(retrieved_host, host);

                Ok(())
            })?
        }
    }
}
