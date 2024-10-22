use dal::{web::AnyWay, *};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{collections::HashMap, fs::File, io::Write, path::PathBuf, str::FromStr};

mod port;

pub use port::HostPort;

use crate::inventory::{Arch, Flavor, Lab};

use crate::allocator::{ResourceHandle, ResourceHandleInner};

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Host {
    pub id: FKey<Host>,
    pub server_name: String,
    pub arch: Arch,
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

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ImportHost {
    pub server_name: String,
    pub arch: Arch,
    pub flavor: String, // Flavor used during provisioning
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

impl ImportHost {
    pub async fn to_host(&self, transaction: &mut EasyTransaction<'_>, proj_path: PathBuf) -> Host {
        let mut flavor_path = proj_path.clone();
        //get back to inventory dir
        flavor_path.pop();
        flavor_path.pop();
        flavor_path.push("flavors");
        flavor_path.push(self.flavor.as_str());
        flavor_path.set_extension("json");

        let flavor = match Flavor::import(transaction, flavor_path.clone(), None).await {
            Ok(of) => match of {
                Some(f) => f.id,
                None => panic!("Imported flavor does not exist"),
            },
            Err(e) => panic!("Failed to import flavor at '{flavor_path:?}' due to error: {e:?}"),
        };

        Host {
            id: FKey::new_id_dangling(),
            server_name: self.server_name.clone(),
            arch: self.arch,
            flavor,
            serial: self.serial.clone(),
            ipmi_fqdn: self.ipmi_fqdn.clone(),
            iol_id: self.iol_id.clone(),
            ipmi_mac: self.ipmi_mac,
            ipmi_user: self.ipmi_user.clone(),
            ipmi_pass: self.ipmi_pass.clone(),
            fqdn: self.fqdn.clone(),
            projects: self.projects.clone(),
            sda_uefi_device: self.sda_uefi_device.clone(),
        }
    }

    pub async fn from_host(transaction: &mut EasyTransaction<'_>, host: &Host) -> ImportHost {
        let clone = host.clone();
        let flavor = clone
            .flavor
            .get(transaction)
            .await
            .expect("Expected to get flavor");
        ImportHost {
            server_name: clone.server_name,
            arch: clone.arch,
            flavor: flavor.name.clone(),
            serial: clone.serial,
            ipmi_fqdn: clone.ipmi_fqdn,
            iol_id: clone.iol_id,
            ipmi_mac: clone.ipmi_mac,
            ipmi_user: clone.ipmi_user,
            ipmi_pass: clone.ipmi_pass,
            fqdn: clone.fqdn,
            projects: clone.projects,
            sda_uefi_device: clone.sda_uefi_device,
        }
    }
}

impl Importable for Host {
    async fn import(
        transaction: &mut EasyTransaction<'_>,
        import_file_path: std::path::PathBuf,
        proj_path: Option<PathBuf>,
    ) -> Result<Option<ExistingRow<Self>>, anyhow::Error> {
        let lab = match Lab::get_by_name(
            transaction,
            proj_path
                .clone()
                .expect("Expected project path")
                .file_name()
                .expect("Expected to find file name")
                .to_str()
                .expect("Expected host data dir for project to have a valid name")
                .to_owned(),
        )
        .await
        {
            Ok(opt_l) => {
                match opt_l {
                    Some(l) => l.id,
                    None => {
                        // In future import labs and try again
                        return Err(anyhow::Error::msg("Specified lab does not exist"));
                    }
                }
            }
            Err(_) => return Err(anyhow::Error::msg("Failed to find specified lab")),
        };
        let host_info: Value = serde_json::from_reader(File::open(import_file_path)?)?;

        let importhost: ImportHost = serde_json::from_value(
            host_info
                .get("host")
                .expect("Expected to get host from host info")
                .clone(),
        )
        .expect("Expected to serialize ImportHost");

        let host_connections: Vec<HostPort> = serde_json::from_value(
            host_info
                .get("connections")
                .expect("Expected to get host from host info")
                .clone(),
        )
        .expect("Expected to serialize ImportHost");

        for port in host_connections.clone() {
            match port.id.get(transaction).await {
                Ok(mut p) => p.mass_update(port).expect("Expected to update HostPort"),
                Err(_) => {
                    NewRow::new(port)
                        .insert(transaction)
                        .await
                        .expect("Expected to insert new HostPort");
                }
            }
        }

        let mut host: Host = importhost
            .to_host(transaction, proj_path.expect("Expected project path"))
            .await;

        if let Ok(mut orig_host) = Host::get_by_name(transaction, host.server_name.clone()).await {
            host.id = orig_host.id;
            orig_host.mass_update(host).unwrap();
            orig_host
                .update(transaction)
                .await
                .expect("Expected to update row");

            let orig_connections = HostPort::all_for_host(transaction, orig_host.id)
                .await
                .expect("Expected to find ports for host");
            for port in orig_connections {
                if !host_connections.contains(&port) {
                    port.id
                        .get(transaction)
                        .await
                        .expect("Expected to get HostPort")
                        .delete(transaction)
                        .await
                        .expect("Expected to remove old HostPort");
                }
            }

            Ok(Some(orig_host))
        } else {
            let res = NewRow::new(host.clone())
                .insert(transaction)
                .await
                .expect("Expected to create new row");

            let _rh =
                ResourceHandle::add_resource(transaction, ResourceHandleInner::Host(res), lab)
                    .await
                    .expect("Couldn't create tracking handle for vlan");
            match res.get(transaction).await {
                Ok(_) => todo!(),
                Err(e) => Err(anyhow::Error::msg(format!(
                    "Failed to import host due to error: {}",
                    e
                ))),
            }
        }
    }

    async fn export(&self, transaction: &mut EasyTransaction<'_>) -> Result<(), anyhow::Error> {
        let res_handle = ResourceHandle::handle_for_host(transaction, self.id)
            .await
            .expect("Expected to find handle for host");
        let lab_name = res_handle
            .lab
            .expect("Expected handle to have lab")
            .get(transaction)
            .await
            .expect("Expected to find lab")
            .name
            .clone();

        let mut host_file_path = PathBuf::from(format!(
            "./config_data/laas-hosts/inventory/labs/{}/hosts/{}",
            lab_name, self.server_name
        ));
        host_file_path.set_extension("json");

        let mut host_file = File::create(host_file_path).expect("Expected to create host file");

        let import_host = ImportHost::from_host(transaction, self).await;

        let import_connection_list = HostPort::all_for_host(transaction, self.id)
            .await
            .expect("Expected to find host");

        let host_info = serde_json::Value::from_str(
            format!(
                "{{\"host\": {}, \"connections\": {}}}",
                serde_json::to_string_pretty(&import_host)?,
                serde_json::to_string_pretty(&import_connection_list)?
            )
            .as_str(),
        )
        .expect("Expected to serialize host info");

        match host_file.write_all(serde_json::to_string_pretty(&host_info)?.as_bytes()) {
            Ok(_) => Ok(()),
            Err(_) => Err(anyhow::Error::msg(format!(
                "Failed to export host {}",
                self.server_name.clone()
            ))),
        }
    }
}

impl DBTable for Host {
    fn id(&self) -> ID {
        self.id.into_id()
    }

    fn table_name() -> &'static str {
        "hosts"
    }

    fn from_row(row: tokio_postgres::Row) -> Result<ExistingRow<Self>, anyhow::Error> {
        Ok(ExistingRow::from_existing(Self {
            id: row.try_get("id")?,
            server_name: row.try_get("server_name")?,
            arch: serde_json::from_value(row.try_get("arch")?)?,
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
            ("arch", Box::new(serde_json::to_value(clone.arch)?)),
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
    pub async fn ports(&self, t: &mut EasyTransaction<'_>) -> Result<Vec<HostPort>, anyhow::Error> {
        let r = HostPort::all_for_host(t, self.id).await;

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
