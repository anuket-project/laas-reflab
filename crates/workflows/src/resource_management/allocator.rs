use dal::{web::*, *};
use dashmap::DashMap;
use models::allocator::*;
use models::{dashboard::*, inventory::Lab};

use models::{
    dashboard::Aggregate,
    inventory::{Flavor, Host, Vlan},
};
use serde::{Deserialize, Serialize};

// TODO: better tracing in this module
use tracing::warn;

pub struct Allocator {
    token: AllocatorToken,

    /// the set of hosts that have been given out within
    /// the past 2 seconds and so should not be re-tried to
    /// allocate for a bit to allow transactions to
    /// commit contention-free
    cooldown: std::sync::Arc<DashMap<FKey<ResourceHandle>, ID>>,
    //lock: std::sync::Mutex<ClientPair>,
    lock: tokio::sync::Mutex<()>,
}

type Request = ResourceRequest;
type Response = ResourceResponse;

impl Allocator {
    fn new() -> Self {
        Self {
            cooldown: Default::default(),
            token: ResourceHandle::get_allocator_token().expect("allocator isn't alone!"),
            lock: tokio::sync::Mutex::new(
                (), //client.expect("couldn't establish long-running connection to DB"),
            ),
        }
    }
}

impl Allocator {
    pub fn instance() -> &'static Self {
        &LL_ALLOCATOR
    }

    pub async fn get_free_hosts(
        &self,
        t: &mut EasyTransaction<'_>,
        lab: FKey<Lab>,
    ) -> Result<Vec<(ExistingRow<Host>, ExistingRow<ResourceHandle>)>, anyhow::Error> {
        //let handles = ResourceHandle::get_free(&self.token, t, ResourceClass::Host).await?;
        let handles =
            ResourceHandle::query_free::<Host>(t, lab, None, None, &[], &Vec::new()).await?;

        let mut r = Vec::new();

        for (h, rh) in handles.into_iter() {
            let h = h.get(t).await?;
            let rh = rh.get(t).await?;

            r.push((h, rh));
        }

        Ok(r)
    }

    pub async fn get_free_vlans(
        &self,
        t: &mut EasyTransaction<'_>,
        lab: FKey<Lab>,
    ) -> Result<Vec<(ExistingRow<Vlan>, ExistingRow<ResourceHandle>)>, anyhow::Error> {
        //let handles = ResourceHandle::get_free(&self.token, t, ResourceClass::Host).await?;
        let handles =
            ResourceHandle::query_free::<Vlan>(t, lab, None, None, &[], &Vec::new()).await?;

        let mut r = Vec::new();

        for (h, rh) in handles.into_iter() {
            let h = h.get(t).await?;
            let rh = rh.get(t).await?;

            r.push((h, rh));
        }

        Ok(r)
    }

    /// Should never panic, as it is called with an exclusive allocator lock held
    pub async fn deallocate_aggregate(
        &self,
        t: &mut EasyTransaction<'_>,
        agg: FKey<Aggregate>,
    ) -> Result<(), anyhow::Error> {
        tracing::info!("Deallocating aggregate {agg:?}");
        ResourceHandle::deallocate_all(&self.token, t, agg).await
    }

    /// Should never panic, as it is called with an exclusive allocator lock held
    pub async fn deallocate_host(
        &self,
        t: &mut EasyTransaction<'_>,
        host: ResourceHandle,
        agg: FKey<Aggregate>,
    ) -> Result<(), anyhow::Error> {
        ResourceHandle::deallocate_one(&self.token, t, Some(agg), host.id).await?;

        Ok(())
    }

    /// Should never panic, as it is called with an exclusive allocator lock held
    /// `fake` indicates that no cooldown should be applied, and that this is just an
    /// availability try
    pub async fn allocate_host(
        &self,
        t: &mut EasyTransaction<'_>,
        flavor: FKey<Flavor>,
        for_aggregate: FKey<Aggregate>,
        reason: AllocationReason,
        fake: bool,
    ) -> Result<(FKey<Host>, ResourceHandle), anyhow::Error> {
        let _lock = self.lock.lock().await;
        let mut t = t.easy_transaction().await?;

        let lab = match for_aggregate.get(&mut t).await {
            Ok(a) => match a.metadata.lab.clone() {
                Some(lab_name) => match Lab::get_by_name(&mut t, lab_name).await {
                    Ok(lab_res) => match lab_res {
                        Some(l) => l,
                        None => {
                            return Err(anyhow::Error::msg(
                                "Lab does not exist, unable to allocate",
                            ))
                        }
                    },
                    Err(_e) => {
                        return Err(anyhow::Error::msg("Error finding lab, unable to allocate"))
                    }
                },
                None => return Err(anyhow::Error::msg("No lab provided, unable to allocate")),
            },
            Err(e) => return Err(e),
        };

        let res = ResourceHandle::allocate_one(
            &self.token,
            &mut t,
            ResourceRequestInner::HostByFlavor {
                flavor,
                lab: lab.id,
            },
            Some(for_aggregate),
            reason,
            &self.except_resources(),
        )
        .await
        .map(|v| v.into_inner());

        match res {
            Ok(
                handle @ ResourceHandle {
                    id: _,
                    tracks,
                    lab: _,
                },
            ) if let ResourceHandleInner::Host(h) = tracks => {
                t.commit().await.map_err(|e| {
                    anyhow::Error::msg(format!(
                        "Couldn't commit allocation of resource, error: {e:?}"
                    ))
                })?;
                if !fake {
                    // add cooldowns again *if necessary*
                    //self.add_cooldown(id);
                }
                Ok((h, handle))
            }
            Ok(v) => {
                let _ = t.rollback(); // let go of the resource, on failure this automatically happens at "some point" anyway
                Err(anyhow::Error::msg(format!(
                    "got wrong type of resource given back to us from allocator: {v:?}"
                )))
            }
            Err(e) => {
                let _ = t.rollback(); // let go of the resource, on failure this automatically happens at "some point" anyway
                Err(e)
            }
        }

        //Self::instance().allocate_host_internal()
    }

    pub async fn allocate_specific_host(
        &self,
        t: &mut EasyTransaction<'_>,
        host: FKey<Host>,
        for_aggregate: FKey<Aggregate>,
        reason: AllocationReason,
    ) -> Result<(FKey<Host>, ResourceHandle), anyhow::Error> {
        let _lock = self.lock.lock().await;
        let mut t = t.easy_transaction().await?;

        let lab = match ResourceHandle::handle_for_host(&mut t, host).await {
            Ok(res) => res.lab,
            Err(e) => return Err(e),
        };

        let res = ResourceHandle::allocate_one(
            &self.token,
            &mut t,
            ResourceRequestInner::SpecificHost {
                host,
                lab: lab.expect("Expected to find specific host!"),
            },
            Some(for_aggregate),
            reason,
            &self.except_resources(),
        )
        .await
        .map(|v| v.into_inner());

        match res {
            Ok(
                handle @ ResourceHandle {
                    id: _,
                    tracks,
                    lab: _,
                },
            ) if let ResourceHandleInner::Host(h) = tracks => {
                t.commit().await.map_err(|e| {
                    anyhow::Error::msg(format!(
                        "Couldn't commit allocation of resource, error: {e:?}"
                    ))
                })?;
                //self.add_cooldown(id);
                Ok((h, handle))
            }
            Ok(v) => {
                let _ = t.rollback(); // let go of the resource, on failure this automatically happens at "some point" anyway
                Err(anyhow::Error::msg(format!(
                    "got wrong type of resource given back to us from allocator: {v:?}"
                )))
            }
            Err(e) => {
                let _ = t.rollback(); // let go of the resource, on failure this automatically happens at "some point" anyway
                Err(e)
            }
        }
    }

    /// Should never panic, as it is called with an exclusive allocator lock held
    pub async fn allocate_vlans_for(
        &self,
        t: &mut EasyTransaction<'_>,
        agg: FKey<Aggregate>,
        networks: Vec<FKey<Network>>,
        within: FKey<NetworkAssignmentMap>,
    ) -> Result<(), anyhow::Error> {
        let _lock = self.lock.lock().await;
        let mut t = t.easy_transaction().await?;

        //self.lock.lock().map_err(|pe| anyhow::Error::msg("allocator lock was poisoned"))?;

        // try and allocate a matching vlan for each asked for,
        // if any fail then release the ones already allocated and return error
        // if all succeed, return the aggregate map

        //let mut map = NetworkAssignmentMap::empty();
        let lab = match agg.get(&mut t).await {
            Ok(a) => a.lab,
            Err(_e) => return Err(anyhow::Error::msg("Error getting aggregate: {e}")),
        };

        tracing::warn!("Lab is {:?}", lab);
        let mut map = within.get(&mut t).await?;
        tracing::warn!("Map is {:?}", map);
        let is_dynamic = lab
            .get(&mut t)
            .await
            .expect("Expected to get lab from fkey")
            .is_dynamic;

        for network in networks {
            let net = network.get(&mut t).await.map_err(|e| anyhow::Error::msg(format!("Failure to get network from fkey, bubbling error so that mutex guard doesn't poison: {e:?}")))?;

            let vlan = match is_dynamic {
                true => None,
                false => {
                    tracing::warn!("Lab is not dynamic. Network is {net:?}");
                    // There is no good way to find out which specific vlan we need without major breaking changes. For now, just use the name of the network.
                    let static_vlan_id = Self::get_static_vlan(&mut t, &net).await;
                    tracing::warn!("Lab is not dynamic. Passing vlan ({static_vlan_id:?})");
                    Some(static_vlan_id)
                }
            };

            let allocation_result = self
                .allocate_vlan_internal(
                    &mut t,
                    Some(agg),
                    net.public,
                    vlan,
                    AllocationReason::ForBooking,
                    lab,
                )
                .await;
            match allocation_result {
                Ok((v, _h)) => {
                    map.add_assignment(network, v);
                }
                Err(e) => {
                    // release all the vlans and error out
                    let res = t.rollback().await;
                    match res {
                        Ok(_) => {return Err(anyhow::Error::msg(format!("Failed to allocate all vlans with error: {e}. Deallocated")))},
                        Err(e2) => {return Err(anyhow::Error::msg(format!("Failed to rollback allocation of vlans with: {e2} after failing to allocate with: {e}")))},
                    }
                }
            }
        }

        map.update(&mut t).await?;

        t.commit().await?;

        Ok(())
    }

    pub async fn get_static_vlan(
        t: &mut EasyTransaction<'_>,
        net: &ExistingRow<Network>,
    ) -> FKey<Vlan> {
        let network_name = net.name.clone();
        let split_index = network_name
            .find(char::is_numeric)
            .unwrap_or(network_name.len());

        let (_, network_number) = network_name.split_at(split_index);

        tracing::warn!("Getting static vlan for {network_number}, full name: {network_name}");
        // jump!

        let network_number = network_number
            .parse::<i16>()
            .expect("expected to convert &str to i16");
        let vlan = Vlan::select()
            .where_field("vlan_id")
            .equals(network_number)
            .run(t)
            .await
            .unwrap()
            .first()
            .expect("Expected to find a vlan from id")
            .id;
        vlan
    }

    pub async fn allocate_vlan(
        &self,
        t: &mut EasyTransaction<'_>,
        for_agg: Option<FKey<Aggregate>>,
        id: Option<FKey<Vlan>>,
        public: bool,
        reason: AllocationReason,
    ) -> Result<(FKey<Vlan>, ResourceHandle), anyhow::Error> {
        let _lock = self.lock.lock().await;
        let mut t = t.easy_transaction().await?;
        let lab = match for_agg.expect("Expected to have an agg").get(&mut t).await {
            Ok(a) => a.lab,
            Err(_e) => return Err(anyhow::Error::msg("Error getting aggregate: {e}")),
        };

        let res = self
            .allocate_vlan_internal(&mut t, for_agg, public, id, reason, lab)
            .await;

        t.commit().await?;

        res
    }

    /// Returns the set of projects a given user should have vpn access to at the present time
    pub async fn active_vpn_for(
        &self,
        t: &mut EasyTransaction<'_>,
        user: String,
    ) -> Result<Vec<String>, anyhow::Error> {
        let _lock = self.lock.lock().await;

        ResourceHandle::active_vpn_for(t, user).await
    }

    pub async fn allocate_vpn(
        &self,
        t: &mut EasyTransaction<'_>,
        for_aggregate: FKey<Aggregate>,
        for_user: String,
        for_project: String,
    ) -> Result<(), anyhow::Error> {
        let _lock = self.lock.lock().await;

        let lab = match Lab::get_by_name(t, for_project.clone()).await {
            Ok(opt_lab) => match opt_lab {
                Some(l) => l.id,
                None => return Err(anyhow::Error::msg("Lab does not exist")),
            },
            Err(_e) => return Err(anyhow::Error::msg("Failed to find lab: {e}")),
        };

        let _vpn = ResourceHandle::allocate_one(
            &self.token,
            t,
            ResourceRequestInner::VPNAccess {
                for_project,
                for_user,
                lab,
            },
            Some(for_aggregate),
            AllocationReason::ForBooking,
            &vec![],
        )
        .await?;

        // don't return anything as the existence of the allocated vpn token is what denotes access
        Ok(())
    }

    /// Should never panic, as it is called with an exclusive allocator lock held
    async fn allocate_vlan_internal(
        &self,
        t: &mut EasyTransaction<'_>,
        agg_id: Option<FKey<Aggregate>>,
        public: bool,
        id: Option<FKey<Vlan>>,
        reason: AllocationReason,
        lab: FKey<Lab>,
    ) -> Result<(FKey<Vlan>, ResourceHandle), anyhow::Error> {
        let inner = match id {
            Some(id) => ResourceRequestInner::SpecificVlan { vlan: id, lab },
            None => ResourceRequestInner::VlanByCharacteristics {
                public,
                serves_dhcp: public,
                lab,
            },
        };

        /*let req = ResourceRequest {
            for_aggregate: agg_id,
            reason: "allocating a VLAN through the interface in liblaas".to_owned(),
            inner,
        };*/

        let except = self.except_resources();
        let resp =
            ResourceHandle::allocate_one(&self.token, t, inner, agg_id, reason, &except).await;

        match resp {
            Ok(h) => match h.tracks {
                ResourceHandleInner::PrivateVlan(v) | ResourceHandleInner::PublicVlan(v) => {
                    Ok((v, h.into_inner()))
                }
                other => {
                    warn!("This should be unreachable!");
                    Err(format!(
                        "we got back an incorrect type of resource: {other:?}"
                    ))
                    .anyway()
                }
            },
            Err(e) => Err(format!("error allocating vlan: {e:?}")).anyway(),
        }
    }

    fn except_resources(&self) -> Vec<FKey<ResourceHandle>> {
        self.cooldown.iter().map(|rm| *rm.key()).collect()
    }

    #[allow(dead_code)]
    fn add_cooldown(&self, res: FKey<ResourceHandle>) {
        let cooldown_token = ID::new();
        self.cooldown.insert(res, cooldown_token);
        let rc = self.cooldown.clone();

        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_secs(2));

            // now, remove it if it's still there and has the same id
            rc.remove_if(&res, |_k, v| *v == cooldown_token);
        });
    }
}

lazy_static::lazy_static! {
    static ref LL_ALLOCATOR: Allocator = Allocator::new();
}

#[derive(Clone, Debug, Serialize, Deserialize, Hash)]
pub struct ResourceRequest {
    pub for_aggregate: FKey<Aggregate>,
    pub inner: ResourceRequestInner,
    pub reason: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, Hash)]
pub struct ResourceResponse {
    pub is_a: ResourceClass,
    pub handle: Result<ResourceHandle, AllocationFailure>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Hash)]
pub enum AllocationFailure {
    NoneAvailable,
    PermissionsDenied,
    DatabaseFailure,
}
