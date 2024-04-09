//! Copyright (c) 2023 University of New Hampshire
//! SPDX-License-Identifier: MIT

use common::prelude::{
    axum::async_trait,
    itertools::Itertools,
    rand::{seq::SliceRandom, thread_rng},
    serde_json::Value,
};
use llid::*;
use dal::{web::*, *};
use tokio_postgres::types::ToSql;

// use core::slice::SlicePattern;
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
};

use super::{dal::*, dashboard::*, inventory::*};

use common::prelude::*;

#[derive(Deserialize, Serialize, Debug, Clone, Hash)]
pub struct ResourceHandle {
    pub id: FKey<ResourceHandle>,
    pub tracks: ResourceHandleInner,
    pub lab: Option<FKey<Lab>>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Hash, Copy)]
pub enum AllocationStatus {
    Allocated,
    Free,
    Broken,
}

#[derive(Serialize, Deserialize, Debug, Clone, Hash)]
pub enum ResourceClass {
    None,
    Host,
    PrivateVlan,
    PublicVlan,
    VPNAccess,
}

inventory::submit! { Migrate::new(VPNToken::migrations) }
#[derive(Serialize, Deserialize, Debug, Clone, Hash)]
// todo - delete this entirely
pub struct VPNToken {
    id: FKey<VPNToken>,
    username: String,
    project: String,
}

impl DBTable for VPNToken {
    fn table_name() -> &'static str {
        "vpn_tokens"
    }

    fn id(&self) -> ID {
        self.id.into_id()
    }

    fn from_row(row: tokio_postgres::Row) -> Result<ExistingRow<Self>, anyhow::Error> {
        let id = row.try_get("id").anyway()?;
        let username = row.try_get("username").anyway()?;
        let project = row.try_get("project").anyway()?;

        Ok(ExistingRow::from_existing(Self {
            id,
            username,
            project,
        }))
    }

    fn to_rowlike(&self) -> Result<HashMap<&str, Box<dyn ToSql + Sync + Send>>, anyhow::Error> {
        let Self {
            id,
            username,
            project,
        } = self.clone();
        let c: [(&str, Box<dyn tokio_postgres::types::ToSql + Sync + Send>); _] = [
            ("id", Box::new(id)),
            ("username", Box::new(username)),
            ("project", Box::new(project)),
        ];

        Ok(c.into_iter().collect())
    }

    fn migrations() -> Vec<Migration> {
        vec![
            Migration {
                unique_name: "create_vpn_tokens_0001",
                description: "create a table for the vpn tokens to track vpn access",
                depends_on: vec![],
                apply: Apply::SQL(format!(
                    "CREATE TABLE IF NOT EXISTS vpn_tokens (
                    id UUID PRIMARY KEY NOT NULL,
                    username VARCHAR NOT NULL,
                    project VARCHAR NOT NULL
                );"
                )),
            },
            Migration {
                unique_name: "create_vpn_tokens_owner_index_0002",
                description: "index to find all vpn tokens for a user",
                depends_on: vec![],
                apply: Apply::SQL(format!(
                    "CREATE INDEX vpn_tokens_owner_index ON vpn_tokens (username);"
                )),
            },
        ]
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, Eq, PartialEq, Hash, Copy)]
pub enum ResourceHandleInner {
    Host(FKey<Host>),
    PrivateVlan(FKey<Vlan>),
    PublicVlan(FKey<Vlan>),
    VPNAccess(FKey<VPNToken>),
    /*VPNAccess {
        access_token_id: llid::LLID,
        username: String,
        project: String,
    },*/
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy)]
pub enum AllocationOperation {
    /// This operation gives out the related handle to be used by a user
    Allocate(),

    /// This operation takes back the related handle and returns it to
    /// the available pool
    Release(),
}

//#[derive(Serialize, Deserialize, Debug, Clone)]
#[derive(Debug, Clone, Copy)]
pub enum AllocationReason {
    /// If a resource is to be used within a booking, allocate
    /// with ForBooking
    ForBooking(),

    /// If a resource is being temporarily taken out of
    /// commission for downtime of some sort,
    /// it should be allocated as ForMaintenance
    ForMaintenance(),

    /// If a resource is being taken out of commission,
    /// it should be allocated with reason ForRetiry
    ForRetiry(),
}

impl Serialize for AllocationReason {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match self {
            Self::ForBooking() => "booking",
            Self::ForMaintenance() => "maintenance",
            Self::ForRetiry() => "retire",
        }
        .to_owned()
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for AllocationReason {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let as_s = String::deserialize(deserializer)?;

        Ok(match as_s.as_str() {
            "booking" => Self::ForBooking(),
            "maintenance" => Self::ForMaintenance(),
            "retire" => Self::ForRetiry(),
            o => Err(serde::de::Error::custom(format!(
                "bad allocation reason: {o}"
            )))?,
        })
    }
}

inventory::submit! { Migrate::new(Allocation::migrations) }
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Allocation {
    pub id: FKey<Allocation>,
    pub for_resource: FKey<ResourceHandle>,
    pub for_aggregate: Option<FKey<Aggregate>>,

    pub started: chrono::DateTime<chrono::Utc>,

    pub ended: Option<chrono::DateTime<chrono::Utc>>,

    pub reason_started: AllocationReason,

    pub reason_ended: Option<String>,
}

impl DBTable for Allocation {
    fn id(&self) -> ID {
        self.id.into_id()
    }

    fn table_name() -> &'static str {
        "allocations"
    }

    fn from_row(row: tokio_postgres::Row) -> Result<ExistingRow<Self>, anyhow::Error> {
        Ok(ExistingRow::from_existing(Self {
            id: row.try_get("id")?,
            for_resource: row.try_get("for_resource")?,
            for_aggregate: row.try_get("for_aggregate")?,
            started: row.try_get("started")?,
            ended: row.try_get("ended")?,

            reason_started: serde_json::from_str(row.try_get("reason_started")?)?,
            reason_ended: row.try_get("reason_ended")?,
        }))
    }

    fn to_rowlike(&self) -> Result<HashMap<&str, Box<dyn ToSql + Sync + Send>>, anyhow::Error> {
        let c: [(&str, Box<dyn tokio_postgres::types::ToSql + Sync + Send>); _] = [
            ("id", Box::new(self.id)),
            ("for_resource", Box::new(self.for_resource)),
            ("for_aggregate", Box::new(self.for_aggregate)),
            ("started", Box::new(self.started)),
            ("ended", Box::new(self.ended)),
            (
                "reason_started",
                Box::new(serde_json::to_string(&self.reason_started)?),
            ),
            ("reason_ended", Box::new(self.reason_ended.clone())),
        ];

        Ok(c.into_iter().collect())
    }

    fn migrations() -> Vec<Migration> {
        vec![
            Migration {
                unique_name: "create_allocations_0001",
                description: "create an allocation table to track allocation of resources",
                depends_on: vec!["create_resource_handles_0001", "create_aggregates_0001"],
                apply: Apply::SQL(format!("CREATE TABLE IF NOT EXISTS allocations (
                            id UUID PRIMARY KEY NOT NULL,
                            for_resource UUID NOT NULL,
                            for_aggregate UUID,
                            started TIMESTAMP WITH TIME ZONE NOT NULL,
                            ended TIMESTAMP WITH TIME ZONE,
                            reason_started VARCHAR NOT NULL,
                            reason_ended VARCHAR,

                            FOREIGN KEY(for_aggregate) REFERENCES aggregates(id) ON DELETE RESTRICT,
                            FOREIGN KEY(for_resource) REFERENCES resource_handles(id) ON DELETE RESTRICT
                );"))
            },
            Migration {
                unique_name: "allocations_check_0002",
                description: "add a sql check statement to prevent two active allocations of the same resource overlapping",
                depends_on: vec!["create_allocations_0001"],
                apply: Apply::SQL(format!("ALTER TABLE allocations ADD CONSTRAINT NoOverlappingAllocations 
                    UNIQUE (for_resource, ended);"))
            }
        ]
    }
}

impl Allocation {
    pub async fn find(
        t: &mut EasyTransaction<'_>,
        for_resource: FKey<ResourceHandle>,
        completed: bool,
    ) -> Result<Vec<ExistingRow<Allocation>>, anyhow::Error> {
        let tn = Self::table_name();
        let q = if completed {
            format!("SELECT * FROM {tn} WHERE ended IS NOT NULL AND for_resource = $1")
        } else {
            format!("SELECT * FROM {tn} WHERE ended IS NULL AND for_resource = $1")
        };

        let rows = t.query(&q, &[&for_resource]).await.anyway()?;

        Allocation::from_rows(rows)
    }

    pub async fn all_for_aggregate(
        t: &mut EasyTransaction<'_>,
        agg: FKey<Aggregate>,
    ) -> Result<Vec<ExistingRow<Allocation>>, anyhow::Error> {
        let tn = Self::table_name();

        let q = format!("SELECT * FROM {tn} WHERE for_aggregate = $1;");

        let rows = t.query(&q, &[&Some(agg)]).await.anyway()?;
        Self::from_rows(rows)
    }
}

/// This struct is intentionally not constructable outside this module,
/// it provides one (and only one!) AT to the blessed allocator
pub struct AllocatorToken {
    #[allow(dead_code)]
    private: (),
}

#[derive(Clone, Debug, Serialize, Deserialize, Hash)]
pub enum ResourceRequestInner {
    VlanByCharacteristics {
        public: bool,
        serves_dhcp: bool,
        lab: FKey<Lab>,
    },

    SpecificVlan {
        vlan: FKey<Vlan>,
        lab: FKey<Lab>,
    },

    HostByFlavor {
        flavor: FKey<Flavor>,
        lab: FKey<Lab>,
    },

    HostByCharacteristics {
        arch: Option<Arch>,
        minimum_ram: Option<DataValue>,
        maximum_ram: Option<DataValue>,
        minimum_cores: Option<DataValue>,
        maximum_cores: Option<DataValue>,
        lab: FKey<Lab>,
    },

    SpecificHost {
        host: FKey<Host>,
        lab: FKey<Lab>,
    },

    VPNAccess {
        for_project: String,
        for_user: String,
        lab: FKey<Lab>,
    },

    /// Deallocates this resource only so long
    /// as the handle is owned by/allocated for
    /// the aggregate in `for_aggregate`
    DeallocateHost {
        resource: ResourceHandle,
    },

    /// Deallocates all resources relating to the given `for_aggregate`
    DeallocateAll {},
}

impl DBTable for ResourceHandle {
    fn table_name() -> &'static str {
        "resource_handles"
    }

    fn id(&self) -> ID {
        self.id.into_id()
    }

    fn from_row(row: tokio_postgres::Row) -> Result<ExistingRow<Self>, anyhow::Error> {
        let id = row.try_get("id").anyway()?;
        let inner = match row.try_get("tracks_resource_type").anyway()? {
            "host" => ResourceHandleInner::Host(row.try_get("tracks_resource").anyway()?),
            "private_vlan" => {
                ResourceHandleInner::PrivateVlan(row.try_get("tracks_resource").anyway()?)
            }
            "public_vlan" => {
                ResourceHandleInner::PublicVlan(row.try_get("tracks_resource").anyway()?)
            }
            "vpn" => ResourceHandleInner::VPNAccess(row.try_get("tracks_resource").anyway()?),
            t => Err(anyhow::Error::msg(format!(
                "bad specifier for resource type '{t}'"
            )))?,
        };
        let lab = row.try_get("lab").anyway()?;

        let s = Self {
            id,
            tracks: inner,
            lab,
        };

        Ok(ExistingRow::from_existing(s))
    }

    fn to_rowlike(&self) -> Result<HashMap<&str, Box<dyn ToSql + Sync + Send>>, anyhow::Error> {
        let (tracking_type, tracking_id, lab) = match self.tracks {
            ResourceHandleInner::Host(h) => ("host", h.into_id(), self.lab),
            ResourceHandleInner::PrivateVlan(id) => ("private_vlan", id.into_id(), self.lab),
            ResourceHandleInner::PublicVlan(id) => ("public_vlan", id.into_id(), self.lab),
            ResourceHandleInner::VPNAccess(id) => ("vpn", id.into_id(), self.lab),
        };

        let c: [(&str, Box<dyn tokio_postgres::types::ToSql + Sync + Send>); _] = [
            ("id", Box::new(self.id)),
            ("tracks_resource", Box::new(tracking_id)),
            ("tracks_resource_type", Box::new(tracking_type)),
            ("lab", Box::new(lab)),
        ];

        Ok(c.into_iter().collect())
    }

    fn migrations() -> Vec<Migration> {
        vec![
            Migration {
                unique_name: "create_resource_handles_0001",
                description: "create the table for resource handles",
                depends_on: vec!["create_hosts_0001", "create_vpn_tokens_0001", "create_vlans_0001"],
                apply: Apply::SQL(format!("CREATE TABLE IF NOT EXISTS resource_handles (
                            id UUID PRIMARY KEY NOT NULL,
                            tracks_resource UUID UNIQUE NOT NULL,
                            tracks_resource_type VARCHAR NOT NULL CHECK (tracks_resource_type IN ('vpn', 'public_vlan', 'private_vlan', 'host'))
                );")),
            },
            Migration {
                unique_name: "add_lab_to_resource_handles_0002",
                description: "add lab field to resource handles",
                depends_on: vec!["create_resource_handles_0001", "create_labs_0002"],
                apply: Apply::SQL(format!("ALTER TABLE resource_handles ADD COLUMN lab UUID;")),
            },

            Migration {
                unique_name: "set_lab_in_resource_handles_0003",
                description: "add lab field to resource handles",
                depends_on: vec!["add_lab_to_resource_handles_0002"],
                apply: Apply::Operation(Box::new(UpsertResourceHandles())),
            },
            Migration {
                unique_name: "set_lab_no_null_0004",
                description: "set lab field to not null",
                depends_on: vec!["set_lab_in_resource_handles_0003"],
                apply: Apply::SQL(format!(
                    "ALTER TABLE resource_handles ALTER COLUMN lab SET NOT NULL;"
                )),
            },
        ]
    }
}

pub struct UpsertResourceHandles();

#[async_trait]
impl ComplexMigration for UpsertResourceHandles {
    async fn run(&self, transaction: &mut EasyTransaction<'_>) -> Result<(), anyhow::Error> {

        let dir = PathBuf::from("./config_data/laas-hosts/inventory");

        let mut proj_vec = dir.read_dir().expect("Expected to read import dir");

        for f in proj_vec {
            let proj = f.unwrap();
            println!("project: {:?}", proj.file_name());
            let lab_name = proj.file_name().to_str().unwrap().to_owned();

            for h in proj.path().read_dir().unwrap() {
                let mut t = transaction.easy_transaction().await.unwrap();
                let host_file = h.unwrap();
                println!("host {:?}", host_file.file_name().to_str().unwrap().split('.').collect_tuple::<(&str, &str)>().unwrap().0.to_owned());

                let host = Host::get_by_name(
                    &mut t,
                    host_file
                        .file_name()
                        .to_str()
                        .unwrap()
                        .to_owned()
                        .split('.')
                        .collect_tuple::<(&str, &str)>()
                        .unwrap()
                        .0
                        .to_owned(),
                )
                .await
                .unwrap();
                let mut binding = ResourceHandle::select()
                    .where_field("tracks_resource")
                    .equals(host.id)
                    .run(&mut t)
                    .await
                    .unwrap();

                let mut handle = binding.pop().unwrap();

                handle.lab = Some(Lab::get_by_name(&mut t, lab_name.clone())
                    .await
                    .unwrap()
                    .unwrap()
                    .id);
                match (handle.update(&mut t)).await {
                    Ok(_) => {
                        t.commit().await.unwrap();
                    }
                    Err(_) => {
                        tracing::error!(
                            "FAILED TO UPSERT RESOURCE HANDLE FOR HOST {}",
                            host.server_name
                        );
                    }
                }
            }
        }

        let mut pub_vlan_handles = ResourceHandle::select()
            .where_field("tracks_resource_type")
            .equals("public_vlan")
            .run(transaction)
            .await
            .unwrap();

        let lab = Lab::select()
            .where_field("name")
            .equals("anuket")
            .run(transaction)
            .await
            .unwrap()
            .get(0)
            .cloned()
            .expect("Expected to find the anuket lab");

        for handle in pub_vlan_handles.iter_mut() {
            handle.lab = Some(lab.id);
            handle
                .update(transaction)
                .await
                .expect(format!("Expected to update {:?}", handle.id).leak());
        }

        let mut vpn_handles = ResourceHandle::select()
            .where_field("tracks_resource_type")
            .equals("vpn")
            .run(transaction)
            .await
            .unwrap();

        for handle in vpn_handles.iter_mut() {
            handle.lab = Some(lab.id);
            handle
                .update(transaction)
                .await
                .expect(format!("Expected to update {:?}", handle.id).leak());
        }

        let mut public_vlan_handles = ResourceHandle::select()
            .where_field("tracks_resource_type")
            .equals("public_vlan")
            .run(transaction)
            .await
            .unwrap();

        for handle in public_vlan_handles.iter_mut() {
            handle.lab = Some(lab.id);
            handle
                .update(transaction)
                .await
                .expect(format!("Expected to update {:?}", handle.id).leak());
        }

        let mut private_vlan_handles = ResourceHandle::select()
            .where_field("tracks_resource_type")
            .equals("private_vlan")
            .run(transaction)
            .await
            .unwrap();

        for handle in private_vlan_handles.iter_mut() {
            handle.lab = Some(lab.id);
            handle
                .update(transaction)
                .await
                .expect(format!("Expected to update {:?}", handle.id).leak());
        }

        let lfedge_lab = Lab::select()
            .where_field("name")
            .equals("lfedge")
            .run(transaction)
            .await
            .unwrap()
            .get(0)
            .cloned()
            .expect("Expected to find the anuket lab");

        let lfedge_vlans = [3001, 3002, 3003, 3004, 3007, 3008, 3015, 3016];


    for vlan_id in lfedge_vlans {
        let lfedge_vlan = Vlan::select()
            .where_field("vlan_id")
            .equals(vlan_id as i16)
            .run(transaction)
            .await
            .unwrap()
            .get(0)
            .cloned()
            .expect("Can't find vlan with id");

        let mut resource_handle = ResourceHandle::select()
            .where_field("tracks_resource")
            .equals(lfedge_vlan.id)
            .run(transaction)
            .await
            .unwrap()
            .get(0)
            .cloned()
            .expect("Expected to find resource handle for vlan!");

        resource_handle.lab = Some(lfedge_lab.id);
        resource_handle.update(transaction).await.expect("Expected to update lab for resource handle!");
    }

        Ok(())
    }
}

lazy_static::lazy_static! {
    static ref TOKEN: std::sync::Mutex<Option<AllocatorToken>> = std::sync::Mutex::new(Some(AllocatorToken { private: () }));
}

inventory::submit! { Migrate::new(ResourceHandle::migrations) }
impl ResourceHandle {
    /// This function allows getting one, single, program wide AllocatorToken
    /// that is to be used by *only* the blessed allocator

    pub fn get_allocator_token() -> Result<AllocatorToken, ()> {
        TOKEN.lock().unwrap().take().ok_or(())
    }

    pub async fn active_vpn_for(
        t: &mut EasyTransaction<'_>,
        user: String,
    ) -> Result<Vec<String>, anyhow::Error> {
        let v_tn = VPNToken::table_name();
        let handles_tn = <ResourceHandle as DBTable>::table_name();
        let allocation_tn = Allocation::table_name();

        /*let q = format!("SELECT DISTINCT {v_tn}.project
        FROM (
                {v_tn}
            INNER JOIN
                {handles_tn}
            ON ({handles_tn}.tracks_resource = {v_tn}.id)
        )
        WHERE
            ({v_tn}.username = $1)
        AND
            EXISTS (
                SELECT * FROM {allocation_tn}
                      WHERE ended IS NULL AND {allocation_tn}.for_resource = {handles_tn}.tracks_resource);");*/

        let mut vpn_tokens: Vec<(FKey<VPNToken>, FKey<ResourceHandle>)> = Vec::new();
        // TODO: Could change to query less and just itereate through all of the labs if query_allocated can handle an Option<Fkey<Lab>>
        for lab in match Lab::select().run(t).await {
            Ok(lab_vec) => lab_vec,
            Err(e) => return Err(anyhow::Error::msg("Error getting labs: {e}")), // this is failing
        } {
            let mut tokens = match Self::query_allocated::<VPNToken>(
                t,
                lab.id,
                Some(format!("{v_tn}.username = $1")),
                None,
                &[&user],
                &vec![],
            )
            .await
            {
                Ok(t) => vpn_tokens.append(&mut t.into_iter().map(|f| f).collect_vec()),
                Err(e) => tracing::debug!("No VPN allocations for lab: {:?}", lab.name),
            };
        }

        let mut projects = HashSet::new();

        for (tfk, _) in vpn_tokens {
            let token = tfk.get(t).await?;
            assert!(token.username == user);
            projects.insert(token.project.clone());
        }

        Ok(projects.into_iter().collect_vec())
    }

    pub async fn allocation_is_allowed(
        transaction: &mut EasyTransaction<'_>,
        rh: FKey<ResourceHandle>,
        message: String,
    ) -> Result<(), anyhow::Error> {
        let allocations = Allocation::find(transaction, rh, false).await?;

        match allocations.len() {
            0 => Ok(()),
            1 => {
                let allocation = &allocations[0];
                let fa = &allocation.for_aggregate;
                let aid = allocation.id;
                tracing::info!("It's already allocated within allocation for agg: {fa:?}, allocation id: {aid:?}");
                Err("already booked").anyway()
            }
            _ => {
                tracing::info!("{message}");
                tracing::info!("Invalid database state, resource was already alloc'd");
                unreachable!("BUG: resource is allocated multiple times, consistency failure")
            }
        }
    }

    /// Expects the first argument to the query this produces to be the FK of
    /// the lab the resource is owned by
    ///
    /// WARNING: this function is gross and kind of a footgun at times,
    /// I recommend carefully tracing the whole `make_query`, `query`, `query_free` and
    /// `query_allocated` call chain before making any tweaks here, as it all tightly integrates
    /// and is the very core part of LibLaaS that avoids double booking hosts or resources
    /// and maintains DB consistency
    fn make_query<T: DBTable>(
        available: bool,
        lab_param_idx: usize,
        filter: Option<String>,
    ) -> String {
        let free = true;

        let additional_filter = if let Some(v) = filter {
            v
        } else {
            format!("TRUE")
        };

        let tn = T::table_name();
        let a_tn = Allocation::table_name();
        let handles_tn = ResourceHandle::table_name();

        tracing::info!("Lab param: {}", lab_param_idx);

        // if ended is null, then it has not yet ended
        // select handles where no allocation exists that hasn't yet ended
        let available_handles = format!("SELECT * FROM {handles_tn}
            WHERE (NOT EXISTS (SELECT * FROM {a_tn} WHERE ended IS NULL AND {a_tn}.for_resource = {handles_tn}.id))
            AND lab = ${lab_param_idx}");

        let allocated_handles = format!("SELECT * FROM {handles_tn}
            WHERE EXISTS (SELECT * FROM {a_tn} WHERE ended IS NULL AND {a_tn}.for_resource = {handles_tn}.id)");

        let handle_set = if available {
            available_handles
        } else {
            allocated_handles
        };

        let g = format!(
            "SELECT resources.id AS resource_id, handles_with_allocs.id AS handle_id FROM (
                    (SELECT * FROM {tn} WHERE {additional_filter}) AS resources
                INNER JOIN
                    ({handle_set}) AS handles_with_allocs
                ON handles_with_allocs.tracks_resource = resources.id
            )"
        );

        tracing::info!(g);
        g
    }

    pub async fn query_free<T: DBTable>(
        t: &mut EasyTransaction<'_>,
        lab: FKey<Lab>,
        filter: Option<String>,
        limit: Option<usize>,
        params: &[&(dyn ToSql + Sync)],
        except_for: &Vec<FKey<ResourceHandle>>,
    ) -> Result<Vec<(FKey<T>, FKey<ResourceHandle>)>, anyhow::Error> {
        Self::query(t, lab, true, filter, limit, params, except_for).await
    }

    pub async fn query_allocated<T: DBTable>(
        t: &mut EasyTransaction<'_>,
        lab: FKey<Lab>,
        filter: Option<String>,
        limit: Option<usize>,
        params: &[&(dyn ToSql + Sync)],
        except_for: &Vec<FKey<ResourceHandle>>,
    ) -> Result<Vec<(FKey<T>, FKey<ResourceHandle>)>, anyhow::Error> {
        Self::query(t, lab, false, filter, limit, params, except_for).await
    }

    pub async fn query<T: DBTable>(
        t: &mut EasyTransaction<'_>,
        lab: FKey<Lab>,
        free: bool,
        filter: Option<String>,
        limit: Option<usize>,
        params: &[&(dyn ToSql + Sync)],
        except_for: &Vec<FKey<ResourceHandle>>,
    ) -> Result<Vec<(FKey<T>, FKey<ResourceHandle>)>, anyhow::Error> {
        let tn = T::table_name();
        tracing::info!("Querying for free {tn}");

        let mut params: Vec<&(dyn ToSql + Sync)> = Vec::from_iter(params.iter().copied());

        params.push(&lab);


        let query = Self::make_query::<T>(free, params.len(), filter);

        tracing::info!("Query that is getting run is\n{query}");

        let v = t.query(&query, params.as_slice()).await?;

        let v = v
            .into_iter()
            .map(|row| (row.get("resource_id"), row.get("handle_id")));

        let v = v.filter(|(tfk, rhfk)| !except_for.contains(&rhfk));

        if let Some(l) = limit {
            Ok(v.take(l).collect_vec())
        } else {
            Ok(v.collect_vec())
        }
    }

    pub async fn find_one_available(
        _token: &AllocatorToken,
        transaction: &mut EasyTransaction<'_>,
        filter: ResourceRequestInner,
        except_for: &Vec<FKey<ResourceHandle>>,
    ) -> Result<ExistingRow<ResourceHandle>, anyhow::Error> {
        match filter {
            ResourceRequestInner::HostByCharacteristics { .. } => {
                todo!("implement filtering by specs")
            }
            ResourceRequestInner::HostByFlavor { flavor, lab } => {
                let host_tn = Host::table_name();
                let handles_tn = <ResourceHandle as DBTable>::table_name();
                let allocation_tn = Allocation::table_name();

                let free_hosts = Self::query_free::<Host>(
                    transaction,
                    lab,
                    Some(format!("{host_tn}.flavor = $1")),
                    None,
                    &[&flavor],
                    except_for,
                )
                .await?;

                /*let free_hosts = format!("
                    SELECT {handles_tn}.id
                    FROM (
                                {handles_tn}
                            INNER JOIN
                                (SELECT * FROM {host_tn} WHERE {host_tn}.flavor = $1) AS flavored_hosts
                            ON (flavored_hosts.id = {handles_tn}.tracks_resource))
                            EXCEPT (SELECT for_resource FROM {allocation_tn} WHERE ended IS NULL);
                ");*/

                tracing::info!("Got to host alloc");

                tracing::info!("Selecting hosts using query:");
                tracing::info!("{free_hosts:?}");

                tracing::info!("With flavor {flavor:?}");
                tracing::info!("With except_for {except_for:?}");

                //let handle_ids = transaction.query(&free_hosts, &[&flavor]).await.anyway()?;

                // We do the except_for filter down here since
                // it is (almost always) a tiny list, and the sql syntax
                // for excluding it is fragile and arcane
                let mut handle_ids = free_hosts
                    .into_iter()
                    .filter(|(hfk, rhfk)| {
                        tracing::info!("Looking at host {:?} for potential filtering", hfk);
                        !except_for.contains(rhfk)
                    })
                    .collect_vec();

                handle_ids.shuffle(&mut thread_rng());

                let selected_id = handle_ids
                    .get(0)
                    .ok_or("no matching host by the given constraints was found")
                    .anyway()?;

                let fk: FKey<ResourceHandle> = selected_id.1;

                let rh = fk.get(transaction).await?;
                //let actual_host = Host::select().where_field("id").equals(rh.tracks)

                Self::allocation_is_allowed(
                    transaction,
                    rh.id,
                    format!("Host {:?} was already allocd?", rh),
                )
                .await
                .expect("was just free!");

                tracing::info!("Allocates host {:?}", rh);
                Ok(rh)

                //tracing::info!("Returns a single host since one was available");
            }
            ResourceRequestInner::VlanByCharacteristics {
                public,
                serves_dhcp: _,
                lab,
            } => {
                let vlan_tn = <Vlan as DBTable>::table_name();
                let handles_tn = <ResourceHandle as DBTable>::table_name();
                let allocation_tn = Allocation::table_name();

                let additional_public_query = if public {
                    format!("({vlan_tn}.public_config IS NOT NULL)")
                } else {
                    format!("({vlan_tn}.public_config IS NULL)")
                };

                /*let q_vlan_ids = format!("
                    SELECT {handles_tn}.id
                    FROM (
                            {handles_tn}
                        INNER JOIN
                            (SELECT id FROM {vlan_tn} WHERE ({additional_public_query})) AS vlan_objects
                        ON vlan_objects.id = {handles_tn}.tracks_resource
                    )
                    EXCEPT (SELECT for_resource FROM {allocation_tn} WHERE ended IS NULL);
                ");*/

                let set = Self::query_free::<Vlan>(
                    transaction,
                    lab,
                    Some(additional_public_query),
                    None,
                    &[],
                    except_for,
                )
                .await?;

                tracing::info!("Returned set is {set:?}");

                let (free_vlan, rhfk) = set
                    .get(0)
                    .ok_or("no matching vlan by the given constraints was found")
                    .anyway()?;

                let rh = rhfk.get(transaction).await?;

                Self::allocation_is_allowed(
                    transaction,
                    rh.id,
                    format!("Pertains to vlan unknown within handle {:?}", rh),
                )
                .await
                .expect("it was just free");

                tracing::info!("Allocating vlan {:?}", rh);

                Ok(rh)
            }

            ResourceRequestInner::SpecificVlan { vlan, lab } => {
                let tn = Self::table_name();

                let actual_vlan = vlan.get(transaction).await?;

                let q = format!("SELECT * FROM {tn} WHERE tracks_resource = $1;");

                //let vlan = transaction.query_opt(&q, &[&vlan]).await.anyway()?;
                let vlans = Self::query_free::<Vlan>(
                    transaction,
                    lab,
                    Some(format!("id = $1")),
                    None,
                    &[&vlan],
                    except_for,
                )
                .await?;
                let (vlan, handle) = vlans.get(0).ok_or("that vlan was not free").anyway()?;

                Self::allocation_is_allowed(
                    transaction,
                    *handle,
                    format!(
                        "Pertains to vlan {} within handle {:?}",
                        actual_vlan.vlan_id, handle
                    ),
                )
                .await
                .expect("bug");

                tracing::info!("Allocating vlan {}", actual_vlan.vlan_id);
                Ok(handle.get(transaction).await?)
            }

            ResourceRequestInner::SpecificHost { host, lab } => {
                let tn = Self::table_name();

                let actual_host = host.get(transaction).await?;

                let q = format!("SELECT * FROM {tn} WHERE tracks_resource = $1;");

                let host = transaction
                    .query_opt(&q, &[&host.into_id()])
                    .await
                    .anyway()?;
                let host = host.ok_or("no matching rh for host found").anyway()?;
                let host = Self::from_row(host)?;

                if let Ok(_) = Self::allocation_is_allowed(
                    transaction,
                    host.id,
                    format!(
                        "Host {} was already allocd?",
                        actual_host.server_name.clone()
                    ),
                )
                .await
                {
                    tracing::info!("Allocates host {}", actual_host.server_name);
                    Ok(host)
                } else {
                    tracing::info!("Host {} was already allocated", actual_host.server_name);
                    Err("host was not available").anyway()
                }
            }

            ResourceRequestInner::VPNAccess {
                for_project,
                for_user,
                lab,
            } => {
                //let tn = <VPNToken as DBTable>::table_name();

                let t = VPNToken {
                    id: FKey::new_id_dangling(),

                    username: for_user,
                    project: for_project,
                };

                let nr = NewRow::new(t);

                let vti = nr.insert(transaction).await?;

                let lab = match Lab::get_by_name(transaction, "anuket".to_string()).await {
                    Ok(o) => match o {
                        Some(lab) => lab.id,
                        None => return Err(anyhow::Error::msg("Lab does not exist".to_string())),
                    },
                    Err(e) => return Err(anyhow::Error::msg(e.to_string())),
                };

                let ri = ResourceHandle::add_resource(
                    transaction,
                    ResourceHandleInner::VPNAccess(vti),
                    lab,
                )
                .await?;

                ri.get(transaction).await
            }

            _other => unreachable!("not an allocation request"),
        }
    }

    pub async fn allocate_one(
        token: &AllocatorToken,
        t: &mut EasyTransaction<'_>,
        filter: ResourceRequestInner,
        for_aggregate: Option<FKey<Aggregate>>,
        reason: AllocationReason,
        except_for: &Vec<FKey<ResourceHandle>>,
    ) -> Result<ExistingRow<ResourceHandle>, anyhow::Error> {
        let mut transaction = t.easy_transaction().await?;

        let r = Self::find_one_available(token, &mut transaction, filter, except_for).await?;

        // now create an AllocationEvent that shows this action occurred
        let ae = Allocation {
            id: FKey::new_id_dangling(),
            for_resource: r.id,

            for_aggregate,

            started: chrono::Utc::now(),
            ended: None,

            reason_started: reason,
            reason_ended: None,
        };

        let nr = NewRow::new(ae);

        nr.insert(&mut transaction).await?;

        transaction
            .commit()
            .await
            .map_err(|e| anyhow::Error::from(e))?;

        Ok(r)
    }

    pub async fn deallocate_one(
        _token: &AllocatorToken,
        t: &mut EasyTransaction<'_>,
        from_aggregate: Option<FKey<Aggregate>>,
        resource: FKey<ResourceHandle>,
    ) -> Result<(), anyhow::Error> {
        let mut transaction = t.easy_transaction().await?;

        let allocation: Option<ExistingRow<Allocation>> =
            Allocation::find(&mut transaction, resource, false)
                .await
                .map_err(|e| {
                    anyhow::Error::msg(format!(
                        "Allocations could not be queried for that resource, error was {e:?}"
                    ))
                })?
                .get(0)
                .cloned();

        if let Some(mut allocation) = allocation {
            if allocation.for_aggregate != from_aggregate {
                return Err(anyhow::Error::msg(
                    "The matching allocation is not from the given aggregate!",
                ));
            }

            tracing::info!(
                "Ending allocation {allocation:?} which was for resource {:?}",
                resource.get(&mut transaction).await.unwrap().into_inner()
            );

            allocation.ended = Some(chrono::Utc::now());

            allocation.update(&mut transaction).await?;

            transaction.commit().await?;

            Ok(())
        } else {
            transaction.rollback().await?;

            Err(anyhow::Error::msg(
                "no live allocation existed for the resource",
            ))
        }
    }

    pub async fn deallocate_all(
        token: &AllocatorToken,
        t: &mut EasyTransaction<'_>,
        from_aggregate: FKey<Aggregate>,
    ) -> Result<(), anyhow::Error> {
        let mut transaction = t.easy_transaction().await?;

        for allocation in Allocation::all_for_aggregate(&mut transaction, from_aggregate).await? {
            if allocation.ended.is_none() {
                tracing::info!("Deallocating allocation {allocation:?}");
                let res = Self::deallocate_one(
                    token,
                    &mut transaction,
                    Some(from_aggregate),
                    allocation.for_resource,
                )
                .await;

                match res {
                    Ok(()) => continue,
                    Err(e) => Err(e)?,
                }
            } else {
                tracing::warn!("Trying to end an allocation that already ended");
            }
        }

        transaction.commit().await?;

        Ok(())
    }

    pub async fn currently_owned_by(
        &self,
        t: &mut EasyTransaction<'_>,
        //resource: ResourceHandle,
        allocated_to: FKey<Aggregate>,
    ) -> Result<bool, anyhow::Error> {
        for allocation in Allocation::all_for_aggregate(t, allocated_to).await? {
            if allocation.for_resource == self.id && allocation.ended.is_none() {
                return Ok(true);
            }
        }

        Ok(false)
    }

    pub async fn handle_for_host(
        transaction: &mut EasyTransaction<'_>,
        host: FKey<Host>,
    ) -> Result<Self, anyhow::Error> {
        let handles_tn = <ResourceHandle as DBTable>::table_name();
        let q = format!("SELECT * FROM {handles_tn} WHERE tracks_resource = $1;");

        let row = transaction.query_opt(&q, &[&host]).await.anyway()?;

        let row = row.ok_or(anyhow::Error::msg(format!(
            "No resource handle existed for host {host:?}"
        )))?;

        let h = Self::from_row(row)?;

        Ok(h.into_inner())
    }

    pub async fn handle_for_vlan(
        transaction: &mut EasyTransaction<'_>,
        vlan: FKey<Vlan>,
    ) -> Result<Self, anyhow::Error> {
        let handles_tn = <ResourceHandle as DBTable>::table_name();
        let q = format!("SELECT * FROM {handles_tn} WHERE tracks_resource = $1;");

        let row = transaction.query_opt(&q, &[&vlan]).await.anyway()?;

        let row = row.ok_or(anyhow::Error::msg(format!(
            "No resource handle existed for host {vlan:?}"
        )))?;

        let h = Self::from_row(row)?;

        Ok(h.into_inner())
    }

    /// Either creates a handle to track the resource,
    /// or returns an error.
    ///
    /// This function checks for duplicate resource inventorying
    /// and emits an error if detected
    pub async fn add_resource(
        transaction: &mut EasyTransaction<'_>,
        resource: ResourceHandleInner,
        lab: FKey<Lab>,
    ) -> Result<FKey<ResourceHandle>, anyhow::Error> {
        NewRow::new(Self {
            id: FKey::new_id_dangling(),
            tracks: resource,
            lab: Some(lab),
        })
        .insert(transaction)
        .await
    }
}
