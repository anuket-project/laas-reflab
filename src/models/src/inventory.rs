//! Copyright (c) 2023 University of New Hampshire
//! SPDX-License-Identifier: MIT

use common::prelude::{itertools::Itertools, macaddr::MacAddr6, *, chrono::{DateTime, Utc}, axum::async_trait, serde_json::Value};
use dal::{
    web::{AnyWay, *},
    *,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use strum_macros::Display;
use tokio_postgres::types::{ToSql, FromSql, private::BytesMut};
use std::{
    collections::HashMap,
    net::{Ipv4Addr, Ipv6Addr}, str::Split, cmp::Ordering,
};

#[derive(Serialize, Deserialize, Debug, Default, Clone, Hash, Copy, JsonSchema)]
pub struct DataValue {
    pub value: u64,
    pub unit: DataUnit,
}

#[derive(Serialize, Deserialize, Debug, Default, Clone, Hash, Copy, JsonSchema)]
pub enum DataUnit {
    #[default]
    Unknown,

    Bytes,
    KiloBytes,
    MegaBytes,
    GigaBytes,
    TeraBytes,

    Bits,

    BitsPerSecond,
    KiloBitsPerSecond,
    MegaBitsPerSecond,
    GigaBitsPerSecond,
}

impl DataValue {
    /// if is_bytes is false, the passed value is instead
    pub fn from_decimal(s: &str, value_type: DataUnit) -> Option<Self> {
        parse_size::parse_size(s)
            .map(|v| DataValue {
                value: v,
                unit: value_type,
            })
            .ok()
    }

    pub fn to_sqlval(&self) -> Result<Box<serde_json::Value>, anyhow::Error> {
        serde_json::to_value(self).map(|v| Box::new(v)).anyway()
    }

    pub fn from_sqlval(v: serde_json::Value) -> Result<Self, anyhow::Error> {
        serde_json::from_value(v).anyway()
    }
}

inventory::submit! { Migrate::new(Flavor::migrations) }
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Flavor {
    pub id: FKey<Flavor>, // Flavor io used to create an instance

    pub arch: Arch,
    pub name: String,
    pub public: bool,
    pub cpu_count: usize,
    pub ram: DataValue,
    pub root_size: DataValue,
    pub disk_size: DataValue,
    pub swap_size: DataValue,
}

impl DBTable for Flavor {
    fn table_name() -> &'static str {
        "flavors"
    }

    fn id(&self) -> ID {
        self.id.into_id()
    }

    fn migrations() -> Vec<Migration> {
        vec![
            Migration {
                unique_name: "create_flavor_0001",
                description: "create the flavor table",
                depends_on: vec![],
                apply: Apply::SQL(format!(
                    "CREATE TABLE IF NOT EXISTS flavors (
                                id UUID PRIMARY KEY NOT NULL,
                                arch VARCHAR NOT NULL,
                                name VARCHAR(1000) UNIQUE NOT NULL,
                                public BOOL NOT NULL,
                                cpu_count JSONB NOT NULL,
                                ram JSONB NOT NULL,
                                root_size JSONB NOT NULL,
                                disk_size JSONB NOT NULL,
                                swap_size JSONB NOT NULL
                    );"
                )),
            },
            Migration {
                unique_name: "index_flavor_0002",
                description: "add index on name column for flavor",
                depends_on: vec!["create_flavor_0001"],
                apply: Apply::SQL(format!("CREATE INDEX flavor_name_index ON flavors (name);")),
            },
        ]
    }

    fn to_rowlike(
        &self,
    ) -> Result<HashMap<&str, Box<dyn ToSqlObject>>, common::prelude::anyhow::Error> {
        let c: [(&str, Box<dyn ToSqlObject>); _] = [
            ("id", Box::new(self.id)),
            ("arch", Box::new(self.arch.to_string())),
            ("name", Box::new(self.name.clone())),
            ("public", Box::new(self.public)),
            (
                "cpu_count",
                Box::new(serde_json::to_value(self.cpu_count as i64)?),
            ),
            ("ram", self.ram.to_sqlval()?),
            ("root_size", self.root_size.to_sqlval()?),
            ("disk_size", self.disk_size.to_sqlval()?),
            ("swap_size", self.swap_size.to_sqlval()?),
        ];

        Ok(c.into_iter().collect())
    }

    fn from_row(
        row: tokio_postgres::Row,
    ) -> Result<ExistingRow<Self>, common::prelude::anyhow::Error> {
        Ok(ExistingRow::from_existing(Self {
            id: row.try_get("id")?,
            arch: Arch::from_string(row.try_get("arch")?)
                .ok_or(anyhow::Error::msg("bad arch parse"))?,
            name: row.try_get("name")?,
            public: row.try_get("public")?,
            cpu_count: serde_json::from_value::<i64>(row.try_get("cpu_count")?)?.min(0) as usize,
            ram: DataValue::from_sqlval(row.try_get("ram")?)?,
            root_size: DataValue::from_sqlval(row.try_get("root_size")?)?,
            disk_size: DataValue::from_sqlval(row.try_get("disk_size")?)?,
            swap_size: DataValue::from_sqlval(row.try_get("swap_size")?)?,
        }))
    }
}

impl Flavor {
    pub async fn get_by_name(
        t: &mut EasyTransaction<'_>,
        name: String,
    ) -> Result<ExistingRow<Flavor>, anyhow::Error> {
        let tn = <Self as DBTable>::table_name();
        let q = format!("SELECT * FROM {tn} WHERE name = $1;");
        let row = t.query_opt(&q, &[&name]).await.anyway()?;
        let row = row.ok_or("no flavor by that name existed").anyway()?;
        let s = Self::from_row(row)?;

        Ok(s)
    }

    pub async fn ports(
        &self,
        transaction: &mut EasyTransaction<'_>,
    ) -> Result<Vec<ExistingRow<InterfaceFlavor>>, anyhow::Error> {
        Ok(InterfaceFlavor::all_for_flavor(transaction, self.id).await?)
    }
}

inventory::submit! { Migrate::new(InterfaceFlavor::migrations) }
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

    fn migrations() -> Vec<Migration> {
        vec![Migration {
            unique_name: "create_interface_flavor_0001",
            description: "create the interface flavor table",
            depends_on: vec!["create_flavor_0001"],
            apply: Apply::SQL(
                "CREATE TABLE IF NOT EXISTS interface_flavors (
                    id UUID PRIMARY KEY NOT NULL,
                    on_flavor UUID NOT NULL,
                    name VARCHAR NOT NULL,
                    speed JSONB NOT NULL,
                    cardtype JSONB NOT NULL,
                    FOREIGN KEY(on_flavor) REFERENCES flavors(id) ON DELETE CASCADE
                );"
                .to_owned(),
            ),
        },
        ]
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
        Ok(Self::from_rows(rows)?)
    }
}

#[derive(Serialize, Deserialize, Debug, Default, Clone, Hash, Copy, JsonSchema)]
pub enum CardType {
    PCIeOnboard,
    PCIeModular,

    #[default]
    Unknown,
}

inventory::submit! { Migrate::new(ExtraFlavorInfo::migrations) }
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

    fn migrations() -> Vec<Migration> {
        vec![
            Migration {
                unique_name: "create_extraflavorinfo_0001",
                description: "create a table to account for non-standard flavor information outside of the main table",
                depends_on: vec!["create_flavor_0001"],
                apply: Apply::SQL("CREATE TABLE IF NOT EXISTS extra_flavor_info (
                    id UUID PRIMARY KEY NOT NULL,
                    for_flavor UUID NOT NULL,
                    extra_trait VARCHAR NOT NULL,
                    key VARCHAR NOT NULL,
                    value VARCHAR NOT NULL,
                    FOREIGN KEY(for_flavor) REFERENCES flavors(id) ON DELETE CASCADE
                );".to_owned()),
            }
        ]
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

inventory::submit! { Migrate::new(Host::migrations) }
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
}

impl DBTable for Host {
    fn id(&self) -> ID {
        self.id.into_id()
    }

    fn migrations() -> Vec<Migration> {
        vec![
            Migration {
                unique_name: "create_hosts_0001",
                description: "create the host table",
                depends_on: vec!["create_flavor_0001"],
                apply: Apply::SQL(format!(
                    "CREATE TABLE IF NOT EXISTS hosts (
                                id UUID PRIMARY KEY NOT NULL,
                                server_name VARCHAR(1000) UNIQUE NOT NULL,
                                arch JSONB NOT NULL,
                                flavor UUID NOT NULL,
                                serial VARCHAR(1000) NOT NULL,
                                ipmi_fqdn VARCHAR(1000) NOT NULL,
                                iol_id VARCHAR(1000) NOT NULL,
                                ipmi_mac MACADDR NOT NULL,
                                sp1_mac MACADDR NOT NULL,
                                ipmi_user VARCHAR(1000) NOT NULL,
                                ipmi_pass VARCHAR(1000) NOT NULL,
                                projects JSONB NOT NULL,
                                fqdn VARCHAR NOT NULL,
                                FOREIGN KEY(flavor) REFERENCES flavors(id) ON DELETE NO ACTION
                            );"
                )),
            },
            Migration {
                unique_name: "remove_sp1_mac_column_hosts_0002",
                description: "remove the sp1_mac column from host table since it is obsolete",
                depends_on: vec!["create_hosts_0001"],
                apply: Apply::SQL(format!("ALTER TABLE hosts DROP COLUMN sp1_mac;")),
            },
        ]
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

#[derive(Serialize, Deserialize, Clone, Debug, Hash, Copy)]
pub enum Arch {
    X86,
    X86_64,
    Aarch64,
}

impl Arch {
    pub fn to_string(&self) -> String {
        match self {
            Arch::X86 => return "x86".to_string(),
            Arch::X86_64 => return "x86_64".to_string(),
            Arch::Aarch64 => return "aarch64".to_string(),
        }
    }

    pub fn from_string(s: String) -> Option<Arch> {
        match s.as_str() {
            "x86" => Some(Arch::X86),
            "x86_64" => Some(Arch::X86_64),
            "aarch64" => Some(Arch::Aarch64),
            _ => None,
        }
    }
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

impl LabStatusInner {

}

inventory::submit! { Migrate::new(Lab::migrations) }
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Lab {
    pub id: FKey<Lab>,
    pub name: String,
    pub location: String,
    pub email: String,
    pub phone: String,
    pub is_dynamic: bool
}

impl Lab {
    pub async fn status(&self) -> Option<FKey<LabStatus>> {
        let mut client = new_client().await.expect("Expected to connect to db");
        let mut transaction = client

        .easy_transaction()
        .await
        .expect("Transaction creation error");
        let stati = LabStatus::select().where_field("for_lab").equals(self.id).run(&mut transaction).await.expect("Statuses for lab not found");
        match stati.len() {
            0 | 1 => {
                return
                    match stati.get(0) {
                        Some(s) => Some(s.id),
                        None => None
                    }
            },
            _ => {
                let mut largest: ExistingRow<LabStatus> = stati.get(0).expect("Expected to have a lab status").clone();
                for status in stati {
                    if largest.time.cmp(&status.time) == Ordering::Less {
                        largest = status;
                    }
                }
                return Some(largest.id)
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
        let clone = self.clone();

        let id = serde_json::to_value(clone.id)?;
        let name = serde_json::to_value(clone.name)?;
        let location = serde_json::to_value(clone.location)?;
        let email = serde_json::to_value(clone.email)?;
        let phone = serde_json::to_value(clone.phone)?;
        let is_dynamic = serde_json::to_value(clone.is_dynamic)?;


        let c: [(&str, Box<dyn ToSqlObject>); _] = [
            ("id", Box::new(self.id)),
            ("name", Box::new(self.name.clone())),
            ("location", Box::new(self.location.clone())),
            ("email", Box::new(self.email.clone())),
            ("phone", Box::new(self.phone.clone())),
            ("is_dynamic", Box::new(self.is_dynamic.clone())),

        ];

        Ok(c.into_iter().collect())
    }

    fn migrations() -> Vec<Migration> {
        vec![Migration {
            unique_name: "create_labs_0001",
            description: "create sql model for labs",
            depends_on: vec![],
            apply: Apply::SQL(format!(
                "CREATE TABLE IF NOT EXISTS labs (
                        id UUID PRIMARY KEY NOT NULL,
                        name VARCHAR NOT NULL,
                        location VARCHAR NOT NULL,
                        email VARCHAR NOT NULL,
                        phone VARCHAR NOT NULL,
                        is_dynamic BOOLEAN NOT NULL
            );"
            )),
        },
        Migration {
            unique_name: "create_labs_0002",
            description: "Import labs",
            depends_on: vec!["create_labs_0001"],
            apply: Apply::Operation(Box::new(UpsertLabs())),
        }
        ]
    }
}

pub struct UpsertLabs();

#[async_trait]
impl ComplexMigration for UpsertLabs {
    async fn run(&self, transaction: &mut EasyTransaction<'_>) -> Result<(), anyhow::Error> {
        let projects = config::settings().projects.clone();

    tracing::info!("Got client for upsert labs");

    for (name, config) in projects {
        // upsert the lab

        let lab = Lab::get_by_name(transaction, name.clone()).await;

        match lab {
            Ok(lab) => {

                match lab {
                    Some(mut lab) => {
                        // Lab exists, update it regardless if anything changed or not
                        lab.location = config.location;
                        lab.email = config.email;
                        lab.phone = config.phone;

                        lab.update(transaction).await.unwrap();

                        tracing::info!("Updated existing lab: {:?}", lab);
                    },
                    None => {
                        // Lab does not exist, create it
                        let lab = Lab {
                            id: FKey::new_id_dangling(),
                            name: name.clone(),
                            location: config.location,
                            email: config.email,
                            phone: config.phone,
                            is_dynamic: config.is_dynamic,
                        };

                        let res = NewRow::new(lab.clone())
                        .insert(transaction)
                        .await.expect("Failed to insert lab into db!");
                        
                        tracing::info!("Added new lab: {:?}", lab.clone());
                    },
                }
            },
            Err(e) => {
                tracing::error!("{}", format!("{:?}", e));
                return Err(e);
            },
        }
    }
        Ok(())
    }
}

inventory::submit! { Migrate::new(LabStatus::migrations) }
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
            expected_next_event_time: serde_json::from_value(row.try_get("expected_next_event_time")?)?,
            status: serde_json::from_value(row.try_get("status")?)?,
            headline: row.try_get("headline")?,
            subline: row.try_get("subline")?,
        }))
    }
    
    fn to_rowlike(&self) -> Result<HashMap<&str, Box<dyn ToSqlObject>>, anyhow::Error> {
        let clone = self.clone();

        let c: [(&str, Box<dyn ToSqlObject>); _] = [
            ("id", Box::new(self.id)),
            ("for_lab", Box::new(self.for_lab)),
            ("time", Box::new(self.time)),
            ("expected_next_event_time", Box::new(serde_json::to_value(self.expected_next_event_time.clone())?)),
            ("status", Box::new(serde_json::to_value(self.status.clone())?)),
            ("headline", Box::new(self.headline.clone())),
            ("subline", Box::new(self.subline.clone())),
        ];

        Ok(c.into_iter().collect())
    }

    fn migrations() -> Vec<Migration> {
        vec![Migration {
            unique_name: "create_lab_statuses_0001",
            description: "create sql model for lab statuses",
            depends_on: vec![],
            apply: Apply::SQL(format!(
                "CREATE TABLE IF NOT EXISTS lab_statuses (
                        id UUID PRIMARY KEY NOT NULL,
                        for_lab UUID NOT NULL,
                        time TIMESTAMP NOT NULL,
                        expected_next_event_time JSONB NOT NULL,
                        status JSONB NOT NULL,
                        headline VARCHAR,
                        subline VARCHAR
            );"
            )),
        }]
    }
}


inventory::submit! { Migrate::new(Switch::migrations) }
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Switch {
    pub id: FKey<Switch>,

    pub name: String,
    pub ip: String,
    pub user: String,
    pub pass: String,
    pub switch_os: Option<FKey<SwitchOS>>,
    pub management_vlans: Vec<i16>,
    pub ipmi_vlan: i16,
    pub public_vlans: Vec<i16>
}

impl PartialEq for Switch {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id && self.name == other.name && self.ip == other.ip && self.user == other.user && self.pass == other.pass && self.switch_os == other.switch_os
    }
}

impl DBTable for Switch {
    fn id(&self) -> ID {
        self.id.into_id()
    }

    fn table_name() -> &'static str {
        "switches"
    }
    // JSONMODEL -> DBTABLE
    fn from_row(row: tokio_postgres::Row) -> Result<ExistingRow<Self>, anyhow::Error> {
        Ok(ExistingRow::from_existing(Self {
            id: row.try_get("id")?,
            name: row.try_get("name")?,
            ip: row.try_get("ip")?,
            user: row.try_get("switch_user")?,
            pass: row.try_get("switch_pass")?,
            switch_os: row.try_get("switch_os")?,
            management_vlans: row.try_get("management_vlans")?,
            ipmi_vlan: row.try_get("ipmi_vlan")?,
            public_vlans: row.try_get("public_vlans")?,
        }))
    }

    fn to_rowlike(&self) -> Result<HashMap<&str, Box<dyn ToSqlObject>>, anyhow::Error> {
        let clone = self.clone();
        let c: [(&str, Box<dyn ToSqlObject>); _] = [
            ("id", Box::new(clone.id)),
            ("name", Box::new(clone.name)),
            ("ip", Box::new(clone.ip)),
            ("switch_user", Box::new(clone.user)),
            ("switch_pass", Box::new(clone.pass)),
            ("switch_os", Box::new(clone.switch_os)),
            ("management_vlans", Box::new(clone.management_vlans)),
            ("ipmi_vlan", Box::new(clone.ipmi_vlan)),
            ("public_vlans", Box::new(clone.public_vlans)),
        ];

        Ok(c.into_iter().collect())
    }

    fn migrations() -> Vec<Migration> {
        vec![
            Migration { 
                unique_name: "create_switches_0001",
                description: "Create switches table",
                depends_on: vec![],
                apply: Apply::SQL(format!(
                    "CREATE TABLE IF NOT EXISTS switches (
                            id UUID PRIMARY KEY NOT NULL,
                            data JSONB NOT NULL
                    );"
                )),
            },
            Migration { 
                unique_name: "migrate_switches_0002",
                description: "Migrates the switch table",
                depends_on: vec!["create_switches_0001"],
                apply: Apply::SQLMulti(vec![
                    "ALTER TABLE switches ADD COLUMN name VARCHAR;".to_owned(),
                    "UPDATE switches SET name = data ->> 'name';".to_owned(),
                    "ALTER TABLE switches ALTER COLUMN name SET NOT NULL;".to_owned(),

                    "ALTER TABLE switches ADD COLUMN ip VARCHAR;".to_owned(),
                    "UPDATE switches SET ip = data ->> 'ip';".to_owned(),
                    "ALTER TABLE switches ALTER COLUMN ip SET NOT NULL;".to_owned(),

                    "ALTER TABLE switches ADD COLUMN switch_user VARCHAR;".to_owned(),
                    "UPDATE switches SET switch_user = data ->> 'user';".to_owned(),
                    "ALTER TABLE switches ALTER COLUMN switch_user SET NOT NULL;".to_owned(),

                    "ALTER TABLE switches ADD COLUMN switch_pass VARCHAR;".to_owned(),
                    "UPDATE switches SET switch_pass = data ->> 'pass';".to_owned(),
                    "ALTER TABLE switches ALTER COLUMN switch_pass SET NOT NULL;".to_owned(),

                    "ALTER TABLE switches ADD COLUMN switch_type VARCHAR;".to_owned(),
                    "UPDATE switches SET switch_type = data ->> 'switch_type';".to_owned(),
                    "ALTER TABLE switches ALTER COLUMN switch_type SET NOT NULL;".to_owned(),

                    "ALTER TABLE IF EXISTS switches DROP COLUMN data;".to_owned(),
                ]),
            },
            Migration { 
                unique_name: "add_switch_os_0003",
                description: "Adds the Switch OS column",
                depends_on: vec!["migrate_switches_0002"],
                apply: Apply::SQLMulti(vec![
                    "ALTER TABLE switches ADD COLUMN switch_os UUID;".to_owned(),
                ]),
            },
            Migration {
                unique_name: "add_vlan_support_0004",
                description: "Adds management, public, and ipmi vlans for switches",
                depends_on: vec!["add_switch_os_0003"],
                apply: Apply::SQLMulti(vec![
                    "ALTER TABLE switches ADD COLUMN management_vlans SMALLINT[];".to_owned(),
                    "ALTER TABLE switches ADD COLUMN ipmi_vlan SMALLINT;".to_owned(),
                    "ALTER TABLE switches ADD COLUMN public_vlans SMALLINT[];".to_owned(),
                ]),
            },
            Migration {
                unique_name: "wip_switch_type_removal_0005",
                description: "Allowing switch_type to be mull",
                depends_on: vec!["add_vlan_support_0004"],
                apply: Apply::SQLMulti(vec![
                    "ALTER TABLE switches ALTER COLUMN switch_type DROP NOT NULL;".to_owned(),
                ]),
            },
            Migration {
                unique_name: "set_vlan_defaults_0006",
                description: "Sets default values for vlans on existing switches",
                depends_on: vec!["wip_switch_type_removal_0005"],
                apply: Apply::SQLMulti(vec![
                    "UPDATE switches SET management_vlans = '{0}';".to_owned(),
                    "UPDATE switches SET ipmi_vlan = 0;".to_owned(),
                    "UPDATE switches SET public_vlans = '{0}';".to_owned(),
                ]),
            },
            Migration { //Comment for first run if migrating with aggregates using string origins
                unique_name: "drop_switch_type_0010",
                description: "Drops the switch type in favor of the switch os column",
                depends_on: vec!["migrate_switches_0002", "create_switch_os_0001"],
                apply: Apply::SQLMulti(vec![
                    "ALTER TABLE IF EXISTS switches DROP COLUMN switch_type;".to_owned(),
                    "ALTER TABLE switches ALTER COLUMN management_vlans SET NOT NULL;".to_owned(),
                    "ALTER TABLE switches ALTER COLUMN ipmi_vlan SET NOT NULL;".to_owned(),
                    "ALTER TABLE switches ALTER COLUMN public_vlans SET NOT NULL;".to_owned(),
                ]),
            },
        ]
    }
}

impl Switch {
    pub async fn get_by_ip(
        transaction: &mut EasyTransaction<'_>,
        ip: String,
    ) -> Result<Option<ExistingRow<Switch>>, anyhow::Error> {
        let tn = <Self as DBTable>::table_name();
        let q = format!("SELECT * FROM {tn} WHERE ip = $1;");
        let opt_row = transaction.query_opt(&q, &[&ip]).await.anyway()?;
        Ok(match opt_row {
            None => None,
            Some(row) => Some(Self::from_row(row)?),
        })
    }

    pub async fn get_by_name(
        transaction: &mut EasyTransaction<'_>,
        name: String,
    ) -> Result<Option<ExistingRow<Switch>>, anyhow::Error> {
        let tn = <Self as DBTable>::table_name();
        let q = format!("SELECT * FROM {tn} WHERE name = $1;");

        let opt_row = transaction.query_opt(&q, &[&name]).await.anyway()?;
        Ok(match opt_row {
            None => None,
            Some(row) => Some(Self::from_row(row)?),
        })
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum Version {
    Nxos(NxosVersion),
    Sonic(SonicVersion),
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct NxosVersion {
    major: i32,
    minor: i32,
    maintenance: i32,
    train: String,
    rebuild: i32,
}

impl NxosVersion {
    pub fn to_string(&self) -> String {
        return format!("{}.{}({}){}{}", self.major, self.minor, self.maintenance, self.train, self.rebuild)
    }

    pub fn from_string(s: String) -> Result<NxosVersion, Box<dyn std::error::Error + Sync + Send>> {
        if s.contains('.') { // 7.0(3)I3(1)
                let mut version_nums = s.split(['.', '(', ')']);
                // 7 0 3 I3 1
                let major = match verion_num_from_split(&mut version_nums) {
                    Ok(m) => m,
                    Err(e) => return Err(e),
                };
                let minor = match verion_num_from_split(&mut version_nums) {
                    Ok(m) => m,
                    Err(e) => return Err(e),
                };
                let maintenance = match verion_num_from_split(&mut version_nums) {
                    Ok(m) => m,
                    Err(e) => return Err(e),
                };
                let mut train: String = "".to_string();
                let mut rebuild: i32 = 0;

                match version_nums.next() {
                    Some(st) => {
                        if st.len() == 2 {
                            let temp = st.split_at(1);
                            train = temp.0.to_string();
                            rebuild = match i32::from_str_radix(temp.1, 10) {
                                Ok(r) => r,
                                Err(e) => return Err(Box::new(e)),
                            }
                        } else if st.len() == 1 {
                            train = st.to_string();
                            rebuild = 0;
                        }
                    },
                    None => {
                        train = "".to_string();
                        rebuild = 0;
                    }
                };
    
                Ok(NxosVersion {
                    major,
                    minor,
                    maintenance,
                    train,
                    rebuild,
                }
            )
        } else {
            return Err("version format incorrect, missing major or minor".into())
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SonicVersion {
    year: i32,
    month: i8, // If there are somehow more than 256 months in a year please add this to the list of falsehoods programmers believe about time
}

impl SonicVersion {
    pub fn to_string(&self) -> String {
        return format!("{}{:02}", self.year, self.month)
    }

    pub fn from_string(s: String) -> Result<SonicVersion, Box<dyn std::error::Error + Sync + Send>> {
        let version_nums = s.split_at(4);
            Ok(SonicVersion {
                year: match i32::from_str_radix(version_nums.0, 10) {
                    Ok(m) => m,
                    Err(e) => return Err(Box::new(e)),
                },
                month: match i8::from_str_radix(version_nums.1, 10) {
                    Ok(m) => m,
                    Err(e) => return Err(Box::new(e)),
                },
            })
    }
}

impl Version {
    pub fn to_string(&self) -> String {
        match self {
            Version::Nxos(v) => v.to_string(),
            Version::Sonic(v) => v.to_string(),
        }
    }

    pub fn from_string(s: String) -> Result<Version, Box<dyn std::error::Error + Sync + Send>> {
        match s {
            _ if s.contains('.') && s.contains('(') && s.contains(')') => {
                match NxosVersion::from_string(s) {
                    Ok(v) => {Ok(Version::Nxos(v))},
                    Err(e) => {Err(e)}
                }
            },
            _ if s.len() == 6 => {
                match SonicVersion::from_string(s) {
                    Ok(v) => {Ok(Version::Sonic(v))},
                    Err(e) => {Err(e)}
                }
            },
            _ => Err("version format is not supported".into()),
        }
    }

    pub fn eq(self, other: Version) -> bool{
        self.to_string().eq(&other.to_string())
    }
}

impl ToSql for Version {
    fn to_sql(&self, ty: &tokio_postgres::types::Type, out: &mut BytesMut) -> Result<tokio_postgres::types::IsNull, Box<dyn std::error::Error + Sync + Send>>
    where
        Self: Sized {
        self.to_string().to_sql(ty, out)
    }

    fn to_sql_checked(
        &self,
        ty: &tokio_postgres::types::Type,
        out: &mut BytesMut,
    ) -> Result<tokio_postgres::types::IsNull, Box<dyn std::error::Error + Sync + Send>> {
        self.to_string().to_sql_checked(ty, out)
    }

    fn accepts(ty: &tokio_postgres::types::Type) -> bool
    where
        Self: Sized {
            <String as ToSql>::accepts(ty)
    }
}

impl FromSql<'_> for Version {
    fn from_sql<'a>(ty: &tokio_postgres::types::Type, raw: &'a [u8]) -> Result<Self, Box<dyn std::error::Error + Sync + Send>> {
        match Version::from_string(match String::from_sql(ty, raw) {
            Ok(s) => s,
            Err(e) => return Err(e),
        }) {
            Ok(v) => return Ok(v),
            Err(e) => return Err(e),
        }
    }

    fn accepts(ty: &tokio_postgres::types::Type) -> bool {
        <String as FromSql>::accepts(ty)
    }
}

fn verion_num_from_split(spl: &mut Split<'_, [char; 3]>) -> Result<i32, Box<dyn std::error::Error + Sync + Send>> {
    return match spl.next() {
        Some(st) => match i32::from_str_radix(st, 10) {
            Ok(m) => Ok(m),
            Err(e) => Err(Box::new(e)),
        },
        None => Err("version format incorrect".into()),
    }
}

inventory::submit! { Migrate::new(SwitchOS::migrations) }
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SwitchOS {
    pub id: FKey<SwitchOS>,
    pub os_type: String,
    pub version: Version,
}

impl DBTable for SwitchOS {
    fn table_name() -> &'static str {
        "switch_os"
    }

    fn id(&self) -> ID {
        self.id.into_id()
    }
    // JSONMODEL -> DBTABLE
    fn from_row(row: tokio_postgres::Row) -> Result<ExistingRow<Self>, anyhow::Error> {
        Ok(ExistingRow::from_existing(Self {
            id: row.try_get("id")?,
            os_type: row.try_get("os_type")?,
            version: row.try_get("version")?,
        }))
    }

    fn to_rowlike(&self) -> Result<HashMap<&str, Box<dyn ToSqlObject>>, anyhow::Error> {
        let clone = self.clone();
        let c: [(&str, Box<dyn ToSqlObject>); _] = [
            ("id", Box::new(clone.id)),
            ("os_type", Box::new(clone.os_type)),
            ("version", Box::new(clone.version)),
        ];

        Ok(c.into_iter().collect())
    }

    fn migrations() -> Vec<Migration> {
        vec![
            Migration { 
                unique_name: "create_switch_os_0001",
                description: "create switch os table",
                depends_on: vec![],
                apply: Apply::SQL(format!(
                    "CREATE TABLE IF NOT EXISTS switch_os (
                            id UUID PRIMARY KEY NOT NULL,
                            os_type VARCHAR NOT NULL,
                            version VARCHAR NOT NULL
                    );"
                )),
            }
        ]
    }
}

inventory::submit! { Migrate::new(HostPort::migrations) }
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct HostPort {
    pub id: FKey<HostPort>,

    pub on_host: FKey<Host>,
    pub switchport: Option<FKey<SwitchPort>>,
    pub name: String,
    pub speed: DataValue,
    pub mac: MacAddr6,
    pub switch: String,
    pub bus_addr: String,

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
        let speed: serde_json::Value = row.try_get("speed")?;
        let speed: DataValue = match speed {
            v => serde_json::from_value(v)?,
        };
        let mac: serde_json::Value = row.try_get("mac")?;
        let mac: MacAddr6 = match mac {
            v => serde_json::from_value(v)?,
        };

        Ok(ExistingRow::from_existing(Self {
            id: row.try_get("id")?,
            on_host: row.try_get("on_host")?,
            switchport: row.try_get("switchport")?,
            name: row.try_get("name")?,
            speed,
            mac,
            switch: row.try_get("switch")?,
            bus_addr: row.try_get("bus_addr")?,
            is_a: row.try_get("is_a")?,
        }))
    }

    fn to_rowlike(&self) -> Result<HashMap<&str, Box<dyn ToSqlObject>>, anyhow::Error> {
        let clone = self.clone();

        let speed = serde_json::to_value(clone.speed)?;
        let mac = serde_json::to_value(clone.mac)?;

        let c: [(&str, Box<dyn ToSqlObject>); _] = [
            ("id", Box::new(self.id)),
            ("on_host", Box::new(self.on_host)),
            ("switchport", Box::new(self.switchport)),
            ("name", Box::new(clone.name)),
            ("speed", Box::new(speed)),
            ("mac", Box::new(mac)),
            ("switch", Box::new(clone.switch)),
            ("bus_addr", Box::new(clone.bus_addr)),
            ("is_a", Box::new(self.is_a)),
        ];

        Ok(c.into_iter().collect())
    }

    fn migrations() -> Vec<Migration> {
        vec![Migration {
            unique_name: "create_host_ports_0001",
            description: "create sql model for host_ports",
            depends_on: vec!["create_hosts_0001", "create_interface_flavor_0001"],
            apply: Apply::SQL(format!(
                "CREATE TABLE IF NOT EXISTS host_ports (
                        id UUID PRIMARY KEY NOT NULL,
                        on_host UUID NOT NULL,
                        switchport UUID NOT NULL,
                        name VARCHAR NOT NULL,
                        speed JSONB NOT NULL,
                        mac JSONB NOT NULL,
                        switch VARCHAR NOT NULL,
                        bus_addr VARCHAR NOT NULL,
                        is_a UUID NOT NULL,
                        FOREIGN KEY(on_host) REFERENCES hosts(id) ON DELETE CASCADE
            );"
            )),
        }]
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

inventory::submit! { Migrate::new(SwitchPort::migrations) }
#[derive(Serialize, Deserialize, Debug, Clone, Hash)]
pub struct SwitchPort {
    pub id: FKey<SwitchPort>,

    pub for_switch: FKey<Switch>,
    pub name: String,
}

impl DBTable for SwitchPort {
    fn id(&self) -> ID {
        self.id.into_id()
    }

    fn table_name() -> &'static str {
        "switchports"
    }
    // JSONMODEL -> DBTABLE
    fn from_row(row: tokio_postgres::Row) -> Result<ExistingRow<Self>, anyhow::Error> {

        Ok(ExistingRow::from_existing(Self {
            id: row.try_get("id")?,
            for_switch: row.try_get("for_switch")?,
            name: row.try_get("name")?,
        }))
    }

    fn to_rowlike(&self) -> Result<HashMap<&str, Box<dyn ToSqlObject>>, anyhow::Error> {
        let clone = self.clone();

        let c: [(&str, Box<dyn ToSqlObject>); _] = [
            ("id", Box::new(clone.id)),
            ("for_switch", Box::new(clone.for_switch)),
            ("name", Box::new(clone.name)),
        ];

        Ok(c.into_iter().collect())
    }

    fn migrations() -> Vec<Migration> {
        vec![
            Migration { 
                unique_name: "create_switchports_0001",
                description: "Creates the switchports table",
                depends_on: vec![],
                apply: Apply::SQL(format!(
                    "CREATE TABLE public.switchports (
                        id UUID NOT NULL,
                        data JSONB NOT NULL
                    );"
                )),
            },
            Migration { 
                unique_name: "migrate_switchports_0002",
                description: "Migrates the switchport table",
                depends_on: vec!["create_switchports_0001"],
                apply: Apply::SQLMulti(vec![
                    "ALTER TABLE switchports ADD COLUMN for_switch VARCHAR;".to_owned(),
                    "UPDATE switchports SET for_switch = data ->> 'for_switch';".to_owned(),
                    "ALTER TABLE switchports ALTER COLUMN for_switch SET NOT NULL;".to_owned(),

                    "ALTER TABLE switchports ADD COLUMN name VARCHAR;".to_owned(),
                    "UPDATE switchports SET name = data ->> 'name';".to_owned(),
                    "ALTER TABLE switchports ALTER COLUMN name SET NOT NULL;".to_owned(),

                    "ALTER TABLE IF EXISTS switchports DROP COLUMN data;".to_owned(),

                    "ALTER TABLE switchports ALTER COLUMN for_switch SET DATA TYPE uuid using for_switch::uuid;".to_owned(),
                ]),
            }
        ]
    }
}

impl SwitchPort {
    pub async fn get_or_create_port(
        t: &mut EasyTransaction<'_>,
        on: FKey<Switch>,
        name: String,
    ) -> Result<ExistingRow<SwitchPort>, anyhow::Error> {
        let tn = <Self as DBTable>::table_name();
        let q =
            format!("SELECT * FROM {tn} WHERE for_switch = $1 AND name = $2;");

        let existing = t
            .query_opt(
                &q,
                &[
                    &serde_json::to_value(on).unwrap(),
                    &serde_json::to_value(name.clone()).unwrap(),
                ],
            )
            .await?;

        match existing {
            Some(r) => Ok(Self::from_row(r)?),
            None => {
                // need to create the port
                let sp = SwitchPort {
                    id: FKey::new_id_dangling(),
                    for_switch: on,
                    name,
                };

                Ok(NewRow::new(sp).insert(t).await?.get(t).await?)
            }
        }
    }
}

inventory::submit! { Migrate::new(Vlan::migrations) }
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Vlan {
    pub id: FKey<Vlan>,

    pub vlan_id: i16,
    pub public_config: Option<IPNetwork>,
}

impl DBTable for Vlan {
    fn id(&self) -> ID {
        self.id.into_id()
    }

    fn table_name() -> &'static str {
        "vlans"
    }

    fn from_row(row: tokio_postgres::Row) -> Result<ExistingRow<Self>, anyhow::Error> {
        let pc: Option<serde_json::Value> = row.try_get("public_config")?;
        let pc = match pc {
            Some(v) => Some(serde_json::from_value(v)?),
            None => None,
        };
        Ok(ExistingRow::from_existing(Self {
            id: row.try_get("id")?,
            vlan_id: row.try_get("vlan_id")?,
            public_config: pc,
        }))
    }

    fn to_rowlike(&self) -> Result<HashMap<&str, Box<dyn ToSqlObject>>, anyhow::Error> {
        let clone = self.clone();

        let public_config = match clone.public_config {
            None => None,
            Some(v) => Some(serde_json::to_value(v)?),
        };

        let c: [(&str, Box<dyn ToSqlObject>); _] = [
            ("id", Box::new(self.id)),
            ("vlan_id", Box::new(self.vlan_id)),
            ("public_config", Box::new(public_config)),
        ];

        Ok(c.into_iter().collect())
    }

    fn migrations() -> Vec<Migration> {
        vec![Migration {
            unique_name: "create_vlans_0001",
            description: "create sql model for vlans",
            depends_on: vec![],
            apply: Apply::SQL(format!(
                "CREATE TABLE IF NOT EXISTS vlans (
                        id UUID PRIMARY KEY NOT NULL,
                        vlan_id SMALLINT NOT NULL,
                        public_config JSONB
            );"
            )),
        }]
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, Hash)]
pub struct IPInfo<IP: Serialize + std::fmt::Debug + Clone> {
    pub subnet: IP,
    pub netmask: u8,
    pub gateway: Option<IP>,
    pub provides_dhcp: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct IPNetwork {
    pub v4: Option<IPInfo<Ipv4Addr>>,
    pub v6: Option<IPInfo<Ipv6Addr>>,
}

#[derive(Serialize, Deserialize, Debug, Default, Clone, Hash, Copy)]
pub enum BootTo {
    Network,
    #[default]
    Disk,
}

impl std::fmt::Display for BootTo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Network => write!(f, "Network"),
            Self::Disk => write!(f, "Disk"),
        }
    }
}

inventory::submit! { Migrate::new(Action::migrations) }
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
    fn from_row(row: tokio_postgres::Row) -> Result<ExistingRow<Self>, anyhow::Error> {
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

    fn migrations() -> Vec<Migration> {
        vec![
            Migration { 
                unique_name: "create_host_actions_0001",
                description: "Creates the host_actions table",
                depends_on: vec![],
                apply: Apply::SQL(format!(
                    "CREATE TABLE public.host_actions (
                        id UUID NOT NULL,
                        data JSONB NOT NULL
                    );"
                )),
            },
            Migration { 
                unique_name: "migrate_host_actions_0002",
                description: "Migrates the host_actions table",
                depends_on: vec!["create_host_actions_0001"],
                apply: Apply::SQLMulti(vec![
                    "ALTER TABLE host_actions ADD COLUMN for_host UUID;".to_owned(),
                    "UPDATE host_actions SET for_host = (data ->> 'for_host')::UUID;".to_owned(),
                    "ALTER TABLE host_actions ALTER COLUMN for_host SET NOT NULL;".to_owned(),

                    "ALTER TABLE host_actions ADD COLUMN in_tascii VARCHAR;".to_owned(),
                    "UPDATE host_actions SET in_tascii = data ->> 'in_tascii';".to_owned(),
                    "ALTER TABLE host_actions ALTER COLUMN in_tascii SET NOT NULL;".to_owned(),

                    "ALTER TABLE host_actions ADD COLUMN is_complete BOOLEAN;".to_owned(),
                    "UPDATE host_actions SET in_tascii = (data ->> 'is_complete')::BOOLEAN;".to_owned(),
                    "ALTER TABLE host_actions ALTER COLUMN is_complete SET NOT NULL;".to_owned(),

                    "ALTER TABLE IF EXISTS host_actions DROP COLUMN data;".to_owned(),
                ]),
            },
        ]
    }
}

impl Action {
    pub async fn get_all_incomplete_for_host(
        t: &mut EasyTransaction<'_>,
        host: FKey<Host>,
    ) -> Result<Vec<ExistingRow<Action>>, anyhow::Error> {
        let tn = <Self as DBTable>::table_name();
        let q = format!(
            "SELECT * FROM {tn} WHERE is_complete = $1 AND for_host = $2;"
        );

        let res = t.query(&q, &[&false, &host]).await.anyway()?;

        Ok(Self::from_rows(res)?)
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

        Ok(action.insert(t).await?)
    }
}
