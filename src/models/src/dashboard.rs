//! Copyright (c) 2023 University of New Hampshire
//! SPDX-License-Identifier: MIT

use ::serde_with::serde_as;
use common::prelude::{
    chrono::{DateTime, Utc},
    reqwest::StatusCode,
    serde_json::Value,
};
use dal::{web::*, *};
use std::str::FromStr;
use strum_macros::Display;
use tokio_postgres::types::{FromSql, ToSql};

use common::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap, HashSet};

use crate::inventory::{Flavor, Host, Lab, Vlan};

use super::dal::*;

#[derive(Serialize, Deserialize, Debug, Clone, JsonSchema)]
pub struct AggregateConfiguration {
    pub ipmi_username: String,
    pub ipmi_password: String,
}

inventory::submit! { Migrate::new(Aggregate::migrations) }
#[serde_as]
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Aggregate {
    pub id: FKey<Aggregate>,

    pub deleted: bool,

    pub users: Vec<String>, // the set of users who should have access to this aggregate

    pub vlans: FKey<NetworkAssignmentMap>,

    pub template: FKey<Template>,

    pub metadata: BookingMetadata,

    pub state: LifeCycleState,

    pub configuration: AggregateConfiguration,

    /// The originating project for this aggregate
    pub lab: FKey<Lab>,
}

impl std::fmt::Display for Aggregate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.id.into_id())
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub enum LifeCycleState {
    New,    // signals this booking has not yet been fully provisioned
    Active, // signals this booking is actively being used and has already been provisioned
    // (ready for cleanup, if it's time)
    Done, // signals this booking has been cleaned up and released
}

impl ToSql for LifeCycleState {
    fn to_sql(
        &self,
        ty: &tokio_postgres::types::Type,
        out: &mut tokio_postgres::types::private::BytesMut,
    ) -> Result<tokio_postgres::types::IsNull, Box<dyn std::error::Error + Sync + Send>>
    where
        Self: Sized,
    {
        serde_json::to_value(self)?.to_sql(ty, out)
    }

    fn accepts(ty: &tokio_postgres::types::Type) -> bool
    where
        Self: Sized,
    {
        <serde_json::Value as ToSql>::accepts(ty)
    }

    fn to_sql_checked(
        &self,
        ty: &tokio_postgres::types::Type,
        out: &mut tokio_postgres::types::private::BytesMut,
    ) -> Result<tokio_postgres::types::IsNull, Box<dyn std::error::Error + Sync + Send>> {
        serde_json::to_value(self)?.to_sql_checked(ty, out)
    }
}

impl std::fmt::Display for LifeCycleState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        <Self as std::fmt::Debug>::fmt(self, f)
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct BookingMetadata {
    /// The dashboard booking id
    pub booking_id: Option<String>,
    /// The ipa username of the owner of the booking
    pub owner: Option<String>,
    /// The lab a booking is for
    pub lab: Option<String>,
    /// The purpose of a booking
    pub purpose: Option<String>,
    /// Project a booking belongs to
    pub project: Option<String>,
    /// DateTime<Utc> that contains the start of a booking
    pub start: Option<DateTime<Utc>>,
    /// DateTime<Utc> that contains the end of a booking
    pub end: Option<DateTime<Utc>>,
}

impl DBTable for Aggregate {
    fn id(&self) -> ID {
        self.id.into_id()
    }

    fn table_name() -> &'static str {
        "aggregates"
    }

    fn from_row(row: tokio_postgres::Row) -> Result<ExistingRow<Self>, anyhow::Error> {
        Ok(ExistingRow::from_existing(Self {
            id: row.try_get("id")?,
            deleted: row.try_get("deleted")?,
            users: row.try_get("users")?,
            vlans: row.try_get("vlans")?,
            template: row.try_get("template")?,
            state: serde_json::from_value(row.try_get("lifecycle_state")?)?,
            metadata: serde_json::from_value(row.try_get("metadata")?)?,
            configuration: serde_json::from_value(row.try_get("configuration")?).unwrap_or(
                AggregateConfiguration {
                    ipmi_username: String::new(),
                    ipmi_password: String::new(),
                },
            ),
            lab: row.try_get("lab")?,
        }))
    }

    fn to_rowlike(&self) -> Result<HashMap<&str, Box<dyn ToSqlObject>>, anyhow::Error> {
        let clone = self.clone();
        let c: [(&str, Box<dyn ToSqlObject>); _] = [
            ("id", Box::new(clone.id)),
            ("deleted", Box::new(clone.deleted)),
            ("users", Box::new(clone.users)),
            ("vlans", Box::new(clone.vlans)),
            ("metadata", Box::new(serde_json::to_value(clone.metadata)?)),
            ("template", Box::new(clone.template)),
            ("lifecycle_state", Box::new(clone.state)),
            ("lab", Box::new(clone.lab)),
            (
                "configuration",
                Box::new(serde_json::to_value(clone.configuration)?),
            ),
        ];

        Ok(c.into_iter().collect())
    }

    fn migrations() -> Vec<Migration> {
        vec![
            Migration {
                unique_name: "create_aggregates_0001",
                description: "create sql model for aggregates",
                depends_on: vec!["create_networks_0001", "create_network_assignments_0001"],
                apply: Apply::SQL(format!(
                    "CREATE TABLE IF NOT EXISTS aggregates (
                        id UUID PRIMARY KEY NOT NULL,
                        deleted BOOLEAN NOT NULL,
                        users VARCHAR[] NOT NULL,
                        vlans UUID NOT NULL,
                        metadata JSONB NOT NULL,
                        lifecycle_state JSONB NOT NULL,
                        FOREIGN KEY(vlans) REFERENCES network_assignments(id) ON DELETE RESTRICT,
                        template UUID NOT NULL,
                        FOREIGN KEY(template) REFERENCES templates(id) ON DELETE RESTRICT
            );"
                )),
            },
            Migration {
                unique_name: "add_origin_aggregates_0002",
                description: "add field to track aggregate origin project",
                depends_on: vec!["create_aggregates_0001"],
                apply: Apply::SQLMulti(vec![
                    "ALTER TABLE aggregates ADD COLUMN origin VARCHAR;".to_owned(),
                    "UPDATE aggregates SET origin = 'anuket';".to_owned(),
                    "ALTER TABLE aggregates ALTER COLUMN origin SET NOT NULL;".to_owned(),
                ]),
            },
            Migration {
                unique_name: "add_configuration_aggregates_0003",
                description: "add field to track aggregate-global configuration data",
                depends_on: vec!["add_origin_aggregates_0002"],
                apply: Apply::SQLMulti(vec![
                    "ALTER TABLE aggregates ADD COLUMN configuration JSONB;".to_owned(),
                    "UPDATE aggregates SET configuration = '{}'::json;".to_owned(),
                    "ALTER TABLE aggregates ALTER COLUMN configuration SET NOT NULL;".to_owned(),
                ]),
            },
            Migration {
                unique_name: "rename_origin_to_origin_string_aggregates_0004",
                description: "add field to track aggregate origin project",
                depends_on: vec!["add_configuration_aggregates_0003", "create_labs_0001"],
                apply: Apply::SQLMulti(vec![
                    "ALTER TABLE aggregates RENAME COLUMN origin TO origin_string;".to_owned(),
                    "ALTER TABLE aggregates ADD COLUMN lab UUID;".to_owned(),

                    "UPDATE aggregates SET origin_string = 'reserved' WHERE origin_string = '';".to_owned(),
                ]),
            },
            Migration {
                unique_name: "move_from_origin_string_in_aggregates_0005",
                description: "remove origin_string after setting the correct lab",
                depends_on: vec!["rename_origin_to_origin_string_aggregates_0004"],
                apply: Apply::SQLMulti(vec![
                    "UPDATE aggregates SET lab = (SELECT id FROM labs WHERE name = aggregates.origin_string);".to_owned(),

                    "ALTER TABLE aggregates ALTER COLUMN lab SET NOT NULL;".to_owned(),
                    "ALTER TABLE aggregates DROP COLUMN origin_string;".to_owned(),
                ]),
            },
        ]
    }
}

impl Aggregate {
    pub async fn instances(
        &self,
        t: &mut EasyTransaction<'_>,
    ) -> Result<Vec<ExistingRow<Instance>>, anyhow::Error> {
        Instance::select()
            .where_field("aggregate")
            .equals(self.id)
            .run(t)
            .await
    }
}

inventory::submit! { Migrate::new(Template::migrations) }
#[derive(Serialize, Deserialize, Debug, Clone, JsonSchema)]
pub struct Template {
    pub id: FKey<Template>,
    pub name: String,
    pub deleted: bool,
    pub description: String,
    pub owner: Option<String>,
    pub public: bool,                 // If template should be available to all users
    pub networks: Vec<FKey<Network>>, // User defined network
    pub hosts: Vec<HostConfig>,
    pub lab: FKey<Lab>,
}

impl DBTable for Template {
    fn table_name() -> &'static str {
        "templates"
    }

    fn id(&self) -> ID {
        self.id.into_id()
    }

    fn from_row(row: tokio_postgres::Row) -> Result<ExistingRow<Self>, anyhow::Error> {
        Ok(ExistingRow::from_existing(Self {
            id: row.try_get("id")?,
            owner: row.try_get("owner")?,
            name: row.try_get("name")?,
            deleted: row.try_get("deleted")?,
            public: row.try_get("public")?,
            description: row.try_get("description")?,
            networks: row.try_get("networks")?,
            hosts: serde_json::from_value(row.try_get("hosts")?)?,
            lab: row.try_get("lab")?,
        }))
    }

    fn to_rowlike(&self) -> Result<HashMap<&str, Box<dyn ToSqlObject>>, anyhow::Error> {
        let clone = self.clone();
        let c: [(&str, Box<dyn ToSqlObject>); _] = [
            ("id", Box::new(clone.id)),
            ("owner", Box::new(clone.owner)),
            ("name", Box::new(clone.name)),
            ("deleted", Box::new(clone.deleted)),
            ("public", Box::new(clone.public)),
            ("description", Box::new(clone.description)),
            ("networks", Box::new(clone.networks)),
            ("hosts", Box::new(serde_json::to_value(clone.hosts)?)),
            ("lab", Box::new(clone.lab)),
        ];

        Ok(c.into_iter().collect())
    }

    fn migrations() -> Vec<Migration> {
        vec![
            Migration {
                unique_name: "create_templates_0001",
                description: "create sql model for templates",
                depends_on: vec!["create_networks_0001"],
                apply: Apply::SQL(format!(
                    "CREATE TABLE IF NOT EXISTS templates (
                        id UUID PRIMARY KEY NOT NULL,
                        owner VARCHAR,
                        name VARCHAR NOT NULL,
                        deleted BOOLEAN NOT NULL,
                        public BOOLEAN NOT NULL,
                        lab_name VARCHAR NOT NULL,
                        description VARCHAR NOT NULL,
                        networks UUID[] NOT NULL,
                        hosts JSONB NOT NULL
            );"
                )),
            },
            Migration {
                unique_name: "add_origin_to_templates_0002",
                description: "add origins field to templates",
                depends_on: vec!["create_templates_0001"],
                apply: Apply::SQL(format!("ALTER TABLE IF EXISTS templates ADD origins UUID;")),
            },
            Migration {
                unique_name: "remove_lab_name_from_templates_0003",
                description: "removes lab_name from templates",
                depends_on: vec!["add_origin_to_templates_0002"],
                apply: Apply::SQL(format!(
                    "ALTER TABLE IF EXISTS templates DROP IF EXISTS lab_name;"
                )),
            },
            Migration {
                unique_name: "rename_origins_to_lab_0004",
                description: "renames origins to lab",
                depends_on: vec!["remove_lab_name_from_templates_0003"],
                apply: Apply::SQL(format!(
                    "ALTER TABLE IF EXISTS templates RENAME COLUMN origins TO lab;"
                )),
            },
            Migration {
                unique_name: "set_default_lab_for_templates_0005",
                description: "set all template to 'anuket'",
                depends_on: vec!["rename_origins_to_lab_0004", "create_labs_0001"],
                apply: Apply::SQL(
                    "UPDATE templates SET lab = (SELECT id FROM labs WHERE name = 'anuket');"
                        .to_owned(),
                ),
            },
        ]
    }
}

impl Template {
    pub async fn get_public(t: &mut EasyTransaction<'_>) -> Result<Vec<Template>, anyhow::Error> {
        let table_name = <Template as DBTable>::table_name();

        let query = format!("SELECT * FROM {table_name} WHERE public = $1");
        let qr = t.query(&query, &[&true]).await?;

        let results: Vec<Template> = qr
            .into_iter()
            .filter_map(|row| {
                Template::from_row(row)
                    .log_error(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "database corruption did not allow instantiating a template",
                        true,
                    )
                    .map(|er| er.into_inner())
                    .ok()
            })
            .collect();

        Ok(results)
    }

    pub async fn get_all(t: &mut EasyTransaction<'_>) -> Result<Vec<Template>, anyhow::Error> {
        let table_name = Template::table_name();

        let query = format!("SELECT * FROM {table_name} WHERE deleted = $1;");
        let qr = t.query(&query, &[&false]).await?;

        let results: Vec<Template> = qr
            .into_iter()
            .filter_map(|row| {
                Template::from_row(row)
                    .log_error(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "database corruption did not allow instantiating a template",
                        true,
                    )
                    .map(|er| er.into_inner())
                    .ok()
            })
            .collect();

        Ok(results)
    }

    pub async fn get_by_name(
        t: &mut EasyTransaction<'_>,
        name: String,
    ) -> Result<Vec<ExistingRow<Template>>, anyhow::Error> {
        let table_name = Template::table_name();

        let query = format!("SELECT * FROM {table_name} WHERE name = $1;");
        let rows = t.query(&query, &[&name]).await?;
        let vals: Result<Vec<_>, anyhow::Error> = rows
            .into_iter()
            .map(|row| Template::from_row(row))
            .collect();

        let vals = vals?;

        Ok(vals)
    }

    pub async fn get_by_lab(
        t: &mut EasyTransaction<'_>,
        name: String,
    ) -> Result<Vec<ExistingRow<Template>>, anyhow::Error> {
        let table_name = Template::table_name();

        let query = format!("SELECT * FROM {table_name} WHERE name = $1;");
        let rows = t.query(&query, &[&name]).await?;
        let vals: Result<Vec<_>, anyhow::Error> = rows
            .into_iter()
            .map(|row| Template::from_row(row))
            .collect();

        let vals = vals?;

        Ok(vals)
    }

    pub async fn owned_by(
        t: &mut EasyTransaction<'_>,
        owner: String,
    ) -> Result<Vec<Template>, anyhow::Error> {
        let table_name = Template::table_name();
        let query = format!("SELECT * FROM {table_name} WHERE owner = $1;");

        let qr = t.query(&query, &[&owner]).await.anyway()?;

        let results: Vec<Template> = qr
            .into_iter()
            .filter_map(|row| {
                Template::from_row(row)
                    .log_error(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "database corruption did not allow instantiating a template",
                        true,
                    )
                    .map(|er| er.into_inner())
                    .ok()
            })
            .collect();

        Ok(results)
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, JsonSchema)]
pub struct HostConfig {
    pub hostname: String, // Hostname that the user would like

    pub flavor: FKey<Flavor>,
    pub image: FKey<Image>, // Name of image used to match image id during provisioning
    pub cifile: Vec<FKey<Cifile>>, // A vector of C-I Files. order is determined by order of the Vec

    pub connections: Vec<BondGroupConfig>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Hash, PartialEq, Eq, JsonSchema)]
pub struct VlanConnectionConfig {
    pub network: FKey<Network>,
    pub tagged: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default, JsonSchema)]
pub struct BondGroupConfig {
    pub connects_to: HashSet<VlanConnectionConfig>,
    pub member_interfaces: HashSet<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Hash)]
pub struct NetworkProvData {
    pub network_name: String,
    pub hostname: String,
    pub public: bool,
    pub tagged: bool,
    pub iface: String,
    pub vlan_id: FKey<crate::inventory::Vlan>,
}

inventory::submit! { Migrate::new(Instance::migrations) }
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Instance {
    pub id: FKey<Instance>, // Instance id which exists when the host is being provisioned

    pub within_template: FKey<Template>,

    pub aggregate: FKey<Aggregate>,

    pub network_data: FKey<NetworkAssignmentMap>,

    pub linked_host: Option<FKey<Host>>,

    pub config: HostConfig, // Host config

    pub metadata: HashMap<String, serde_json::Value>,
}

impl std::hash::Hash for Instance {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.id.hash(state);
        self.within_template.hash(state);
    }
}

impl DBTable for Instance {
    fn table_name() -> &'static str {
        "instances"
    }

    fn id(&self) -> ID {
        self.id.into_id()
    }

    fn from_row(row: tokio_postgres::Row) -> Result<ExistingRow<Self>, anyhow::Error> {
        Ok(ExistingRow::from_existing(Self {
            id: row.try_get("id")?,
            within_template: row.try_get("within_template")?,
            aggregate: row.try_get("aggregate")?,
            network_data: row.try_get("network_data")?,
            linked_host: row.try_get("linked_host")?,
            config: serde_json::from_value(row.try_get("config")?)?,
            metadata: serde_json::from_value(row.try_get("metadata")?)?,
        }))
    }

    fn to_rowlike(&self) -> Result<HashMap<&str, Box<dyn ToSqlObject>>, anyhow::Error> {
        let clone = self.clone();
        let c: [(&str, Box<dyn ToSqlObject>); _] = [
            ("id", Box::new(clone.id)),
            ("within_template", Box::new(clone.within_template)),
            ("aggregate", Box::new(clone.aggregate)),
            ("network_data", Box::new(clone.network_data)),
            ("linked_host", Box::new(clone.linked_host)),
            ("config", Box::new(serde_json::to_value(clone.config)?)),
            ("metadata", Box::new(serde_json::to_value(clone.metadata)?)),
        ];

        Ok(c.into_iter().collect())
    }

    fn migrations() -> Vec<Migration> {
        vec![Migration {
            unique_name: "create_instances_0001",
            description: "create sql model for templates",
            depends_on: vec![
                "create_aggregates_0001",
                "create_networks_0001",
                "create_hosts_0001",
                "create_templates_0001",
                "create_network_assignments_0001"],
            apply: Apply::SQL(format!(
                "CREATE TABLE IF NOT EXISTS instances (
                        id UUID PRIMARY KEY NOT NULL,
                        within_template UUID NOT NULL,
                        aggregate UUID NOT NULL,
                        config JSONB NOT NULL,
                        network_data UUID NOT NULL,
                        linked_host UUID,
                        FOREIGN KEY(within_template) REFERENCES templates(id) ON DELETE RESTRICT,
                        FOREIGN KEY(aggregate) REFERENCES aggregates(id) ON DELETE RESTRICT,
                        FOREIGN KEY(network_data) REFERENCES network_assignments(id) ON DELETE RESTRICT,
                        FOREIGN KEY(linked_host) REFERENCES hosts(id) ON DELETE RESTRICT
            );"
            )),
        },
        Migration {
            unique_name: "add_metadata_instances_0002",
            description: "add a metadata field to instances table",
            depends_on: vec!["create_instances_0001"],
            apply: Apply::SQLMulti(vec![
                "ALTER TABLE instances ADD COLUMN metadata JSONB;".to_owned(),
                "UPDATE instances SET metadata = '{}'::json;".to_owned(),
                "ALTER TABLE instances ALTER COLUMN metadata SET NOT NULL;".to_owned(),
            ])
        }
            ]
    }
}

impl Instance {
    pub async fn log(
        inst: FKey<Instance>,
        transaction: &mut EasyTransaction<'_>,
        event: ProvEvent,
        sentiment: Option<StatusSentiment>,
    ) -> Result<(), anyhow::Error> {
        let ple = ProvisionLogEvent {
            id: FKey::new_id_dangling(),
            sentiment: sentiment.unwrap_or(StatusSentiment::unknown),
            instance: inst,
            time: Utc::now(),
            prov_status: event,
        };

        let nr = NewRow::new(ple);

        nr.insert(transaction).await?;

        Ok(())
    }

    pub async fn log_committing(
        inst: FKey<Instance>,
        event: ProvEvent,
        sentiment: Option<StatusSentiment>,
    ) -> Result<(), anyhow::Error> {
        let mut client = new_client().await.log_db_client_error().unwrap();
        let mut transaction = client
            .easy_transaction()
            .await
            .log_db_client_error()
            .unwrap();

        Instance::log(inst, &mut transaction, event, sentiment).await?;
        transaction.commit().await?;

        Ok(())
    }
}

pub trait EasyLog {
    async fn log<H, D>(&self, header: H, detail: D, status: StatusSentiment)
    where
        H: Into<String>,
        D: Into<String>;
}

impl EasyLog for FKey<Instance> {
    async fn log<H, D>(&self, header: H, detail: D, status: StatusSentiment)
    where
        H: Into<String>,
        D: Into<String>,
    {
        let header: String = header.into();
        let detail: String = detail.into();

        tracing::info!("Dispatching log for an instance, header: {header}, detail: {detail}");
        let _ = Instance::log_committing(
            *self,
            ProvEvent {
                event: header,
                details: detail,
            },
            Some(status),
        )
        .await;
    }
}

inventory::submit! { Migrate::new(NetworkAssignmentMap::migrations) }
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct NetworkAssignmentMap {
    pub id: FKey<Self>,

    pub networks: HashMap<FKey<Network>, FKey<Vlan>>,
}

impl NetworkAssignmentMap {
    pub fn empty() -> Self {
        Self {
            id: FKey::new_id_dangling(),
            networks: HashMap::new(),
        }
    }

    pub fn add_assignment(&mut self, net: FKey<Network>, is: FKey<Vlan>) {
        self.networks.insert(net, is);
    }
}

impl DBTable for NetworkAssignmentMap {
    fn id(&self) -> ID {
        self.id.into_id()
    }

    fn table_name() -> &'static str {
        "network_assignments"
    }

    fn from_row(row: tokio_postgres::Row) -> Result<ExistingRow<Self>, anyhow::Error> {
        let networks = row.try_get("networks")?;
        let networks: HashMap<String, String> = serde_json::from_value(networks)?;
        let networks = networks
            .into_iter()
            .filter_map(|(k, v)| {
                let k = ID::from_str(&k).ok()?;
                let v = ID::from_str(&v).ok()?;

                Some((FKey::from_id(k), FKey::from_id(v)))
            })
            .collect();

        Ok(ExistingRow::from_existing(Self {
            id: row.try_get("id")?,
            networks,
        }))
    }

    fn to_rowlike(&self) -> Result<HashMap<&str, Box<dyn ToSqlObject>>, anyhow::Error> {
        let clone = self.clone();
        let networks: HashMap<String, String> = clone
            .networks
            .into_iter()
            .map(|(k, v)| (k.into_id().to_string(), v.into_id().to_string()))
            .collect();
        let networks = serde_json::to_value(networks)?;
        let c: [(&str, Box<dyn ToSqlObject>); _] =
            [("id", Box::new(clone.id)), ("networks", Box::new(networks))];

        Ok(c.into_iter().collect())
    }

    fn migrations() -> Vec<Migration> {
        vec![Migration {
            unique_name: "create_network_assignments_0001",
            description: "create sql model for network assignment maps",
            depends_on: vec!["create_vlans_0001", "create_networks_0001"],
            apply: Apply::SQL(format!(
                "CREATE TABLE IF NOT EXISTS network_assignments (
                        id UUID PRIMARY KEY NOT NULL,
                        networks JSONB NOT NULL
            );"
            )),
        }]
    }
}

inventory::submit! { Migrate::new(Image::migrations) }
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Image {
    pub id: FKey<Image>, // Id of image used for booking

    pub owner: String,
    pub name: String, // Name of image
    pub deleted: bool,
    pub cobbler_name: String,
    pub public: bool,
    pub flavors: Vec<FKey<Flavor>>, // Vector of compatible flavor IDs
}

impl DBTable for Image {
    fn id(&self) -> ID {
        self.id.into_id()
    }

    fn table_name() -> &'static str {
        "images"
    }

    fn from_row(row: tokio_postgres::Row) -> Result<ExistingRow<Self>, anyhow::Error> {
        Ok(ExistingRow::from_existing(Self {
            id: row.try_get("id")?,
            owner: row.try_get("owner")?,
            name: row.try_get("name")?,
            deleted: row.try_get("deleted")?,
            cobbler_name: row.try_get("cobbler_name")?,
            public: row.try_get("public")?,
            flavors: row.try_get("flavors")?,
        }))
    }

    fn to_rowlike(&self) -> Result<HashMap<&str, Box<dyn ToSqlObject>>, anyhow::Error> {
        let clone = self.clone();
        let c: [(&str, Box<dyn ToSqlObject>); _] = [
            ("id", Box::new(clone.id)),
            ("owner", Box::new(clone.owner)),
            ("name", Box::new(clone.name)),
            ("deleted", Box::new(clone.deleted)),
            ("cobbler_name", Box::new(clone.cobbler_name)),
            ("public", Box::new(clone.public)),
            ("flavors", Box::new(clone.flavors)),
        ];

        Ok(c.into_iter().collect())
    }

    fn migrations() -> Vec<Migration> {
        vec![Migration {
            unique_name: "create_image_0001",
            description: "create sql model for images",
            depends_on: vec!["create_flavor_0001"],
            apply: Apply::SQL(format!(
                "CREATE TABLE IF NOT EXISTS images (
                        id UUID PRIMARY KEY NOT NULL,
                        owner VARCHAR NOT NULL,
                        name VARCHAR NOT NULL,
                        deleted BOOLEAN NOT NULL,
                        cobbler_name VARCHAR NOT NULL,
                        public BOOLEAN NOT NULL,
                        flavors UUID[] NOT NULL
            );"
            )),
        }]
    }
}

impl Image {
    pub async fn get_by_name(
        t: &mut EasyTransaction<'_>,
        name: String,
    ) -> Result<ExistingRow<Image>, anyhow::Error> {
        let table_name = Self::table_name();
        let query = format!("SELECT * FROM {table_name} WHERE name = $1;");
        let qr = t.query_opt(&query, &[&name]).await?;
        let qr = qr.ok_or(anyhow::Error::msg("Image did not exist for query"))?;

        let results = Image::from_row(qr)
            .log_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "database corruption did not allow instantiating an image from a row",
                true,
            )
            .map(|i| i.into_inner())
            .unwrap(); // TODO: get rid of unwrap

        Ok(ExistingRow::from_existing(results))
    }

    pub async fn images_for_flavor(
        t: &mut EasyTransaction<'_>,
        flavor: FKey<Flavor>,
        owner: Option<String>,
    ) -> Result<Vec<Image>, anyhow::Error> {
        if owner.is_some() {
            let table_name = Self::table_name();
            let query = format!("SELECT * FROM {table_name} WHERE (owner = $1 OR public = $2) AND ($3 = ANY(flavors));");
            let qr = t.query(&query, &[&owner, &true, &flavor.into_id()]).await?;

            let results: Vec<Image> = qr
                .into_iter()
                .filter_map(|row| {
                    Image::from_row(row)
                        .log_error(
                            StatusCode::INTERNAL_SERVER_ERROR,
                            "database corruption did not allow instantiating an image from a row",
                            true,
                        )
                        .map(|er| er.into_inner())
                        .ok()
                })
                .collect();

            Ok(results)
        } else {
            let table_name = Self::table_name();
            let query =
                format!("SELECT * FROM {table_name} WHERE (public = $1) AND ($2 = ANY(flavors));");
            let qr = t.query(&query, &[&true, &flavor.into_id()]).await?;

            let results: Vec<Image> = qr
                .into_iter()
                .filter_map(|row| {
                    Image::from_row(row)
                        .log_error(
                            StatusCode::INTERNAL_SERVER_ERROR,
                            "database corruption did not allow instantiating an image from a row",
                            true,
                        )
                        .map(|er| er.into_inner())
                        .ok()
                })
                .collect();

            Ok(results)
        }
    }
}

#[allow(non_camel_case_types)]
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Copy)]
pub enum StatusSentiment {
    succeeded,
    in_progress,
    degraded,
    failed,
    unknown,
}

inventory::submit! { Migrate::new(ProvisionLogEvent::migrations) }
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ProvisionLogEvent {
    pub id: FKey<ProvisionLogEvent>,
    pub sentiment: StatusSentiment,
    pub instance: FKey<Instance>,
    pub time: DateTime<Utc>,
    pub prov_status: ProvEvent,
}

impl DBTable for ProvisionLogEvent {
    fn table_name() -> &'static str {
        "provision_log_events"
    }

    fn id(&self) -> ID {
        self.id.into_id()
    }

    fn from_row(row: tokio_postgres::Row) -> Result<ExistingRow<Self>, anyhow::Error> {
        Ok(ExistingRow::from_existing(Self {
            sentiment: row
                .try_get("sentiment")
                .unwrap_or(SqlAsJson(StatusSentiment::unknown))
                .extract(),
            id: row.try_get("id")?,
            instance: row.try_get("instance")?,
            time: row.try_get("time")?,
            prov_status: serde_json::from_value(row.try_get("prov_status")?)?,
        }))
    }

    fn to_rowlike(&self) -> Result<HashMap<&str, Box<dyn ToSqlObject>>, anyhow::Error> {
        let clone = self.clone();
        let c: [(&str, Box<dyn ToSqlObject>); _] = [
            ("id", Box::new(clone.id)),
            ("sentiment", Box::new(SqlAsJson::of(self.sentiment))),
            ("instance", Box::new(clone.instance)),
            ("time", Box::new(clone.time)),
            (
                "prov_status",
                Box::new(serde_json::to_value(clone.prov_status)?),
            ),
        ];

        Ok(c.into_iter().collect())
    }

    fn migrations() -> Vec<Migration> {
        vec![
            Migration {
                unique_name: "create_provision_log_events_0001",
                description: "create sql model for provlogevents",
                depends_on: vec!["create_instances_0001"],
                apply: Apply::SQL(format!(
                    "CREATE TABLE IF NOT EXISTS provision_log_events (
                        id UUID PRIMARY KEY NOT NULL,
                        instance UUID NOT NULL,
                        time TIMESTAMP WITH TIME ZONE NOT NULL,
                        prov_status JSONB NOT NULL,
                        FOREIGN KEY(instance) REFERENCES instances(id) ON DELETE CASCADE
                );"
                )),
            },
            Migration {
                unique_name: "add_sentiment_provision_log_events_0002",
                description: "add sentiment field to provlogevents",
                depends_on: vec!["create_provision_log_events_0001"],
                // leave this nullable, no need to migrate values to a default--default is just
                // unknown, handle on extract
                apply: Apply::SQL(format!(
                    "ALTER TABLE provision_log_events ADD COLUMN sentiment JSONB"
                )),
            },
        ]
    }
}

impl ProvisionLogEvent {
    pub async fn all_for_instance(
        t: &mut EasyTransaction<'_>,
        instance: FKey<Instance>,
    ) -> Result<Vec<ExistingRow<ProvisionLogEvent>>, anyhow::Error> {
        let tn = <Self as DBTable>::table_name();
        let q = format!("SELECT * FROM {tn} WHERE instance = $1;");

        t.query(&q, &[&instance])
            .await
            .map(|rows| Self::from_rows(rows))
            .anyway()
            .flatten()
    }
}

inventory::submit! { Migrate::new(Network::migrations) }
#[derive(Serialize, Deserialize, Debug, Clone, JsonSchema)]
pub struct Network {
    pub id: FKey<Network>,
    pub name: String,
    pub public: bool,
}

impl DBTable for Network {
    fn id(&self) -> ID {
        self.id.into_id()
    }

    fn table_name() -> &'static str {
        "networks"
    }
    // JSONMODEL -> DBTABLE
    fn from_row(row: tokio_postgres::Row) -> Result<ExistingRow<Self>, anyhow::Error> {
        Ok(ExistingRow::from_existing(Self {
            id: row.try_get("id")?,
            name: row.try_get("name")?,
            public: row.try_get("public")?,
        }))
    }

    fn to_rowlike(&self) -> Result<HashMap<&str, Box<dyn ToSqlObject>>, anyhow::Error> {
        let clone = self.clone();
        let c: [(&str, Box<dyn ToSqlObject>); _] = [
            ("id", Box::new(clone.id)),
            ("name", Box::new(clone.name)),
            ("public", Box::new(clone.public)),
        ];

        Ok(c.into_iter().collect())
    }

    fn migrations() -> Vec<Migration> {
        vec![
            Migration {
                unique_name: "create_networks_0001",
                description: "Creates the network table",
                depends_on: vec![],
                apply: Apply::SQL(format!(
                    "CREATE TABLE public.networks (
                        id UUID NOT NULL,
                        data JSONB NOT NULL
                    );"
                )),
            },
            Migration {
                unique_name: "migrate_networks_0002",
                description: "Migrates the network table",
                depends_on: vec![],
                apply: Apply::SQLMulti(vec![
                    "ALTER TABLE networks ADD COLUMN name VARCHAR;".to_owned(),
                    "UPDATE networks SET name = data ->> 'name';".to_owned(),
                    "ALTER TABLE networks ALTER COLUMN name SET NOT NULL;".to_owned(),
                    "ALTER TABLE networks ADD COLUMN public BOOLEAN;".to_owned(),
                    "UPDATE networks SET public = (data ->> 'public')::BOOLEAN;".to_owned(),
                    "ALTER TABLE networks ALTER COLUMN public SET NOT NULL;".to_owned(),
                    "ALTER TABLE IF EXISTS networks DROP COLUMN data;".to_owned(),
                ]),
            },
        ]
    }
}

inventory::submit! { Migrate::new(Cifile::migrations) }
#[derive(Serialize, Deserialize, Debug, Hash, Clone, JsonSchema)]
pub struct Cifile {
    pub id: FKey<Cifile>,
    pub priority: i16,
    pub data: String,
}

impl Cifile {
    pub async fn new(
        t: &mut EasyTransaction<'_>,
        strings: Vec<String>,
    ) -> Result<Vec<FKey<Cifile>>, anyhow::Error> {
        let mut priority: i16 = 1;
        let mut cifiles: Vec<FKey<Cifile>> = Vec::new();
        for data in strings {
            if data != "" {
                let cif = Cifile {
                    id: FKey::new_id_dangling(),
                    priority,
                    data,
                };

                priority += 1; // Starts priority at 2 as the generated file is highest priority

                match NewRow::new(cif.clone()).insert(t).await {
                    Ok(fk) => cifiles.push(fk),
                    Err(e) => {
                        todo!("Handle failure: {e:?}")
                        // TODO
                    }
                }
            }
        }
        Ok(cifiles)
    }
}

impl DBTable for Cifile {
    fn table_name() -> &'static str {
        "ci_files"
    }

    fn id(&self) -> ID {
        self.id.into_id()
    }
    // JSONMODEL -> DBTABLE
    fn from_row(row: tokio_postgres::Row) -> Result<ExistingRow<Self>, anyhow::Error> {
        Ok(ExistingRow::from_existing(Self {
            id: row.try_get("id")?,
            priority: row.try_get("priority")?,
            data: row.try_get("data")?,
        }))
    }

    fn to_rowlike(&self) -> Result<HashMap<&str, Box<dyn ToSqlObject>>, anyhow::Error> {
        let clone = self.clone();
        let c: [(&str, Box<dyn ToSqlObject>); _] = [
            ("id", Box::new(clone.id)),
            ("priority", Box::new(clone.priority)),
            ("data", Box::new(clone.data)),
        ];

        Ok(c.into_iter().collect())
    }
    // id uuid NOT NULL,
    // data jsonb NOT NULL
    fn migrations() -> Vec<Migration> {
        vec![
            Migration {
                unique_name: "create_ci_files_0001",
                description: "Creates the ci file table",
                depends_on: vec![],
                apply: Apply::SQL(format!(
                    "CREATE TABLE public.ci_files (
                        id UUID NOT NULL,
                        data JSONB NOT NULL
                    );"
                )),
            },
            Migration {
                unique_name: "update_ci_files_0002",
                description: "Migrates the ci file table",
                depends_on: vec!["create_ci_files_0001"],
                apply: Apply::SQLMulti(vec![
                    "ALTER TABLE ci_files ADD COLUMN ci_data VARCHAR;".to_owned(),
                    "UPDATE ci_files SET ci_data = data ->> 'data';".to_owned(),
                    "ALTER TABLE ci_files ALTER COLUMN ci_data SET NOT NULL;".to_owned(),
                    "ALTER TABLE ci_files ADD COLUMN priority SMALLINT;".to_owned(),
                    "UPDATE ci_files SET priority = (data ->> 'priority')::SMALLINT;".to_owned(),
                    "ALTER TABLE ci_files ALTER COLUMN priority SET NOT NULL;".to_owned(),
                    "ALTER TABLE IF EXISTS ci_files DROP COLUMN data;".to_owned(),
                    "ALTER TABLE ci_files RENAME COLUMN ci_data TO data;".to_owned(),
                ]),
            },
        ]
    }
}

#[derive(Debug, Display, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
pub enum KeyType {
    Rsa,
    Ed25519,
}

impl KeyType {
    pub fn from_string(s: &str) -> Option<KeyType> {
        match s {
            "Rsa" => return Some(KeyType::Rsa),
            "Ed25519" => return Some(KeyType::Ed25519),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ProvEvent {
    pub event: String,
    pub details: String,
}

impl ProvEvent {
    pub fn new<A, B>(event: A, details: B) -> Self
    where
        A: Into<String>,
        B: Into<String>,
    {
        Self {
            event: event.into(),
            details: details.into(),
        }
    }

    pub fn to_string(&self) -> String {
        format!("{} -- {}", self.event, self.details)
    }
}

#[derive(Debug, Display, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum InfraType {
    Switch,
    Server,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct InstanceProvData {
    pub hostname: String,
    pub flavor: FKey<crate::inventory::Flavor>,
    pub image: String,
    pub cifile: Vec<Cifile>,
    pub ipmi_create: bool,
    pub networks: Vec<NetworkProvData>,
}