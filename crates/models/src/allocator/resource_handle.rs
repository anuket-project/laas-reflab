use anyhow::Result;
use dal::{web::*, *};
use itertools::Itertools;
use rand::{seq::SliceRandom, thread_rng};
use tokio_postgres::types::ToSql;

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

use crate::{
    allocator::{
        vpn_token::VPNToken, Allocation, AllocationReason, AllocatorToken, ResourceRequestInner,
        TOKEN,
    },
    dashboard::Aggregate,
    inventory::*,
};

#[derive(Deserialize, Serialize, Debug, Clone, Hash, Eq, PartialEq, Default)]
pub struct ResourceHandle {
    pub id: FKey<ResourceHandle>,
    pub tracks: ResourceHandleInner,
    pub lab: FKey<Lab>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Eq, PartialEq, Hash, Copy)]
pub enum ResourceHandleInner {
    Host(FKey<Host>),
    PrivateVlan(FKey<Vlan>),
    PublicVlan(FKey<Vlan>),
    VPNAccess(FKey<VPNToken>),
    // VPNAccess {
    //     access_token_id: llid::LLID,
    //     username: String,
    //     project: String,
    // }
}

impl Default for ResourceHandleInner {
    fn default() -> Self {
        ResourceHandleInner::Host(FKey::default())
    }
}

impl DBTable for ResourceHandle {
    fn table_name() -> &'static str {
        "resource_handles"
    }

    fn id(&self) -> ID {
        self.id.into_id()
    }

    fn id_mut(&mut self) -> &mut ID {
        self.id.into_id_mut()
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
}

impl ResourceHandle {
    /// This function allows getting one, single, program wide AllocatorToken
    /// that is to be used by *only* the blessed allocator

    #[allow(clippy::result_unit_err)]
    pub fn get_allocator_token() -> Result<AllocatorToken, ()> {
        TOKEN.lock().unwrap().take().ok_or(())
    }

    pub async fn active_vpn_for(
        t: &mut EasyTransaction<'_>,
        user: String,
    ) -> Result<Vec<String>, anyhow::Error> {
        let v_tn = VPNToken::table_name();
        // let handles_tn = <ResourceHandle as DBTable>::table_name();
        // let allocation_tn = Allocation::table_name();

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
            Err(_e) => return Err(anyhow::Error::msg("Error getting labs: {e}")), // this is failing
        } {
            match Self::query_allocated::<VPNToken>(
                t,
                lab.id,
                Some(format!("{v_tn}.username = $1")),
                None,
                &[&user],
                &[],
            )
            .await
            {
                Ok(t) => vpn_tokens.append(&mut t.into_iter().collect_vec()),
                Err(_e) => tracing::debug!("No VPN allocations for lab: {:?}", lab.name),
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
        let additional_filter = if let Some(v) = filter {
            v
        } else {
            "TRUE".to_string()
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
        except_for: &[FKey<ResourceHandle>],
    ) -> Result<Vec<(FKey<T>, FKey<ResourceHandle>)>, anyhow::Error> {
        Self::query(t, lab, true, filter, limit, params, except_for).await
    }

    pub async fn query_allocated<T: DBTable>(
        t: &mut EasyTransaction<'_>,
        lab: FKey<Lab>,
        filter: Option<String>,
        limit: Option<usize>,
        params: &[&(dyn ToSql + Sync)],
        except_for: &[FKey<ResourceHandle>],
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
        except_for: &[FKey<ResourceHandle>],
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

        let v = v.filter(|(_tfk, rhfk)| !except_for.contains(rhfk));

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
                // let _handles_tn = <ResourceHandle as DBTable>::table_name();
                // let _allocation_tn = Allocation::table_name();

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
                    .first()
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
                // let handles_tn = <ResourceHandle as DBTable>::table_name();
                // let allocation_tn = Allocation::table_name();

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

                let (_free_vlan, rhfk) = set
                    .first()
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
                // let tn = Self::table_name();

                let actual_vlan = vlan.get(transaction).await?;

                // let q = format!("SELECT * FROM {tn} WHERE tracks_resource = $1;");
                //let vlan = transaction.query_opt(&q, &[&vlan]).await.anyway()?;

                let vlans = Self::query_free::<Vlan>(
                    transaction,
                    lab,
                    Some("id = $1".to_string()),
                    None,
                    &[&vlan],
                    except_for,
                )
                .await?;
                let (_vlan, handle) = vlans.first().ok_or("that vlan was not free").anyway()?;

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

            ResourceRequestInner::SpecificHost { host, lab: _ } => {
                let tn = Self::table_name();

                let actual_host = host.get(transaction).await?;

                let q = format!("SELECT * FROM {tn} WHERE tracks_resource = $1;");

                let host = transaction
                    .query_opt(&q, &[&host.into_id()])
                    .await
                    .anyway()?;
                let host = host.ok_or("no matching rh for host found").anyway()?;
                let host = Self::from_row(host)?;

                if Self::allocation_is_allowed(
                    transaction,
                    host.id,
                    format!(
                        "Host {} was already allocd?",
                        actual_host.server_name.clone()
                    ),
                )
                .await
                .is_ok()
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
                lab: _,
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

        transaction.commit().await.map_err(anyhow::Error::from)?;

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
                .first()
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
            lab,
        })
        .insert(transaction)
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use testing_utils::block_on_runtime;

    impl Arbitrary for ResourceHandle {
        type Parameters = ();
        type Strategy = BoxedStrategy<Self>;

        fn arbitrary_with(_args: Self::Parameters) -> Self::Strategy {
            (
                any::<FKey<ResourceHandle>>(), // id
                any::<ResourceHandleInner>(),  // tracks
                any::<FKey<Lab>>(),            // lab
            )
                .prop_map(|(id, tracks, lab)| ResourceHandle { id, tracks, lab })
                .boxed()
        }
    }

    impl Arbitrary for ResourceHandleInner {
        type Parameters = ();
        type Strategy = BoxedStrategy<Self>;

        fn arbitrary_with(_args: Self::Parameters) -> Self::Strategy {
            prop_oneof![
                Just(ResourceHandleInner::Host(FKey::<Host>::new_id_dangling())),
                Just(ResourceHandleInner::PrivateVlan(
                    FKey::<Vlan>::new_id_dangling()
                )),
                Just(ResourceHandleInner::PublicVlan(
                    FKey::<Vlan>::new_id_dangling()
                )),
                Just(ResourceHandleInner::VPNAccess(
                    FKey::<VPNToken>::new_id_dangling()
                )),
            ]
            .boxed()
        }
    }

    proptest! {
        #[test]
        fn test_resource_handle_model(resource_handle in ResourceHandle::arbitrary()) {
            block_on_runtime!({
                let client = new_client().await;
                prop_assert!(client.is_ok(), "DB connection failed: {:?}", client.err());
                let mut client = client.unwrap();

                let transaction = client.easy_transaction().await;
                prop_assert!(transaction.is_ok(), "Transaction creation failed: {:?}", transaction.err());
                let mut transaction = transaction.unwrap();

                let new_row = NewRow::new(resource_handle.clone());
                let insert_result = new_row.insert(&mut transaction).await;
                prop_assert!(insert_result.is_ok(), "Insert failed: {:?}", insert_result.err());

                let retrieved_result = ResourceHandle::select()
                    .where_field("id")
                    .equals(resource_handle.id)
                    .run(&mut transaction)
                    .await;

                prop_assert!(retrieved_result.is_ok(), "Retrieval failed: {:?}", retrieved_result.err());
                let retrieved_handles = retrieved_result.unwrap();

                let retrieved_handle = retrieved_handles.first();
                prop_assert!(retrieved_handle.is_some(), "No Allocation found, empty result");

                let retrieved_handle = retrieved_handle.unwrap().clone().into_inner();
                prop_assert_eq!(retrieved_handle, resource_handle);

                Ok(())

            })?
        }
    }
}
