//! Copyright (c) 2023 University of New Hampshire
//! SPDX-License-Identifier: MIT

use common::prelude::{itertools::Itertools, macaddr::MacAddr6, *};
use dal::{
    web::{AnyWay, *},
    *,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    net::{Ipv4Addr, Ipv6Addr},
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
    pub name: String, // Name of the flavor (used to match a selection to a flavor)
    pub public: bool, // Default True. If template should be available to all users
    pub cpu_count: usize, // Max 4.294967295 Billion
    pub ram: DataValue, // Max 4.294 Petabytes in gig
    pub root_size: DataValue, // Max 4.294 Exabytes in gig
    pub disk_size: DataValue, // Max 4.294 Exabytes in gig
    pub swap_size: DataValue, // Max 9.223372036854775807 Exabytes in gig

                      //TODO: potentially move extra flavor info into an array/json field *on*
                      //flavor, since it is almost never going to be used for a join
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
        }]
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

    /// the list of projects this resource can be allocated for
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
            projects: serde_json::from_value(row.try_get("projects")?)?,
            fqdn: row.try_get("fqdn")?,
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
            (
                "projects",
                Box::new(serde_json::to_value(self.projects.clone())?),
            ),
            ("fqdn", Box::new(self.fqdn.clone())),
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

inventory::submit! { Migrate::new(Switch::migrations) }
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Switch {
    pub id: FKey<Switch>,

    pub name: String,
    pub ip: String,
    pub user: String,
    pub pass: String,
    pub switch_type: String,
}

impl JsonModel for Switch {
    fn id(&self) -> ID {
        self.id.into_id()
    }

    fn table_name() -> &'static str {
        "switches"
    }
}

impl Switch {
    pub async fn get_by_ip(
        transaction: &mut EasyTransaction<'_>,
        ip: String,
    ) -> Result<Option<ExistingRow<Switch>>, anyhow::Error> {
        let ip = serde_json::to_value(ip).unwrap();
        let tn = <Self as DBTable>::table_name();
        let q = format!("SELECT * FROM {tn} WHERE data -> 'ip' = $1;");

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
        let name = serde_json::to_value(name).unwrap();
        let tn = <Self as DBTable>::table_name();
        let q = format!("SELECT * FROM {tn} WHERE data -> 'name' = $1;");

        let opt_row = transaction.query_opt(&q, &[&name]).await.anyway()?;
        Ok(match opt_row {
            None => None,
            Some(row) => Some(Self::from_row(row)?),
        })
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
    pub mac_address: Option<MacAddr6>, // may not need mac address? may want to be option?
}

impl JsonModel for SwitchPort {
    fn id(&self) -> ID {
        self.id.into_id()
    }

    fn table_name() -> &'static str {
        "switchports"
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
            format!("SELECT * FROM {tn} WHERE data -> 'for_switch' = $1 AND data -> 'name' = $2;");

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
                    mac_address: None,
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

impl JsonModel for Action {
    fn id(&self) -> ID {
        self.id.into_id()
    }

    fn table_name() -> &'static str {
        "host_actions"
    }
}

impl Action {
    pub async fn get_all_incomplete_for_host(
        t: &mut EasyTransaction<'_>,
        host: FKey<Host>,
    ) -> Result<Vec<ExistingRow<Action>>, anyhow::Error> {
        let tn = <Self as DBTable>::table_name();
        let q = format!(
            "SELECT * FROM {tn} WHERE data -> 'is_complete' = $1 AND data -> 'for_host' = $2;"
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
