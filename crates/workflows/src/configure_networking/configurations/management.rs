use super::super::{
    types::{BondGroup, NetworkConfig, NetworkConfigBuilder, VlanConnection},
    utils::bondgroup_config_to_bondgroup,
};
use dal::{EasyTransaction, ExistingRow, FKey};
use models::{
    dashboard::Instance,
    inventory::{Host, HostPort},
};
use std::collections::{HashMap, HashSet};
use tracing::info;

pub async fn mgmt_network_config(
    host_id: FKey<Host>,
    t: &mut EasyTransaction<'_>,
) -> NetworkConfig {
    let host = host_id
        .get(t)
        .await
        .expect("host did not exist by given fk?");
    let mut builder = NetworkConfigBuilder::new();
    for port in host.ports(t).await.expect("didn't get ports?") {
        let mut bg = BondGroup::new().with_port(port.id);
        if let Some(bmc_vlan) = port.bmc_vlan_id {
            bg = bg.with_vlan(VlanConnection::from_pair(t, bmc_vlan, true).await);
        }
        if let Some(mgmt_vlan) = port.management_vlan_id {
            bg = bg.with_vlan(VlanConnection::from_pair(t, mgmt_vlan, false).await);
        }
        builder = builder.bond(bg);
    }

    let v = builder.persist(false).build();
    info!("built a network config for the host: {v:#?}");
    v
}

pub async fn mgmt_network_config_with_public(
    host_id: FKey<Host>,
    deployed_as: FKey<Instance>,
    t: &mut EasyTransaction<'_>,
) -> NetworkConfig {
    let h: ExistingRow<Host> = host_id.get(t).await.unwrap();

    let instance = deployed_as.get(t).await.unwrap();

    let network_assignments = instance.network_data.get(t).await.unwrap();

    let mut builder = NetworkConfigBuilder::new();

    let mut configured_ports = HashSet::new();

    for bg_config in instance.config.connections.iter() {
        let ports_by_name: HashMap<String, HostPort> = h
            .ports(t)
            .await
            .expect("didn't get ports")
            .into_iter()
            .map(|p| (p.name.clone(), p))
            .collect();

        let bg = bondgroup_config_to_bondgroup(bg_config, &ports_by_name, &network_assignments, t)
            .await
            .with_vlan(VlanConnection::from_pair(t, 99, true).await)
            .with_vlan(VlanConnection::from_pair(t, 98, false).await);

        for port in bg.member_host_ports.iter() {
            configured_ports.insert(*port);
        }

        builder = builder.bond(bg);
    }

    // set the ports that aren't configured to have nothing on them
    for port in h.ports(t).await.unwrap() {
        if !configured_ports.contains(&port.id) {
            let bg = BondGroup::new().with_port(port.id);
            // don't give the bg any vlans
            builder = builder.bond(bg);
        }
    }

    builder.persist(true).build()
}
