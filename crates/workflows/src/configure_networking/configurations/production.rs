use crate::configure_networking::NetworkConfigBuilder;

use super::super::types::{BondGroup, NetworkConfig, VlanConnection};
use super::super::utils::bondgroup_config_to_bondgroup;
use dal::{EasyTransaction, FKey};
use models::{dashboard::Instance, inventory::Host};
use std::collections::HashSet;

pub async fn prod_network_config(
    host_id: FKey<Host>,
    instance_id: FKey<Instance>,
    t: &mut EasyTransaction<'_>,
) -> NetworkConfig {
    let host = host_id.get(t).await.expect("Unable to find host");
    let instance = instance_id.get(t).await.expect("Unable to find instance");
    let port_map = host
        .ports(t)
        .await
        .expect("Ports missing")
        .into_iter()
        .map(|p| (p.name.clone(), p))
        .collect();

    let mut builder = NetworkConfigBuilder::new();

    // loop through provided/configured connections from the dashboard and add them to the NetworkConfigBuilder
    for conn_config in &instance.config.connections {
        let bond_group = bondgroup_config_to_bondgroup(
            conn_config,
            &port_map,
            &instance.network_data.get(t).await.unwrap(),
            t,
        )
        .await;

        builder = builder.bond(bond_group);
    }

    // collect all the used interfaces/ports to compare in the final step
    let used_ports: HashSet<String> = instance
        .config
        .connections
        .iter()
        .flat_map(|conn| &conn.member_interfaces)
        .cloned()
        .collect();

    // create empty bondgroups for unconfigured ports
    for (port_name, port) in &port_map {
        // if the port was not configured in the dashboard
        if !used_ports.contains(port_name) {
            let mut bondgroup = BondGroup::new();
            // add the port to the bondgroup
            // equivalent to calling `with_port` but doesn't consume the bondgroup
            bondgroup.member_host_ports.push(port.id);

            // add bmc_vlan if present, otherwise do not add any vlans so this interface is
            // disabled/shutdown
            if let Some(vlan_id) = port.bmc_vlan_id {
                let bmc_vlan_connection = VlanConnection::from_pair(t, vlan_id, true).await;
                // I'm purposely not using `with_vlan` because it takes ownership of the bondgroup
                bondgroup.vlans.push(bmc_vlan_connection);
            }

            builder = builder.bond(bondgroup);
        }
    }

    builder.persist(true).build()
}
