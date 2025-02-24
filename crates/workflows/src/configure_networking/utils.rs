use super::types::{BondGroup, VlanConnection};
use dal::EasyTransaction;
use models::inventory::HostPort;
use std::collections::{HashMap, HashSet};

pub async fn bondgroup_config_to_bondgroup(
    bondgroup_config: &models::dashboard::BondGroupConfig,
    interfaces: &HashMap<String, HostPort>,
    networks: &models::dashboard::NetworkAssignmentMap,
    t: &mut EasyTransaction<'_>,
) -> BondGroup {
    let mut bondgroup = BondGroup::new();
    let mut seen_vlans = HashSet::new();

    for port_name in &bondgroup_config.member_interfaces {
        // fetch the HostPort struct from the provided interfaces given its name in the bondgroup_config
        let port = interfaces
            .get(port_name)
            .unwrap_or_else(|| panic!("Interface {} not found", port_name));

        // equivalent to `with_port` but doesn't consume the bondgroup
        bondgroup.member_host_ports.push(port.id);

        // add BMC vlan if present
        if let Some(vlan_id) = port.bmc_vlan_id {
            let bmc_vlan = VlanConnection::from_pair(t, vlan_id, true).await;
            // this just ensures that we don't add the same vlan twice, (if the insert into the
            // `HashSet` returns true, it means the vlan was not already in the set)
            if seen_vlans.insert(bmc_vlan.vlan) {
                bondgroup.vlans.push(bmc_vlan);
            }
        }
    }

    // add pre-configured VLAN connections
    for vlan_config in &bondgroup_config.connects_to {
        // we have to get the actual vlan by searching the NetworkAssignmentMap with the provided
        // Network from the BondGroupConfig
        let vlan = networks
            .networks
            .get(&vlan_config.network)
            .unwrap_or_else(|| panic!("Network {:?} not found", vlan_config.network));

        // build the vlan connection
        let vlan_conn = VlanConnection {
            vlan: *vlan,
            tagged: vlan_config.tagged,
        };

        // if we haven't already, push the VlanConnection to the bondgroup
        if seen_vlans.insert(vlan_conn.vlan) {
            bondgroup.vlans.push(vlan_conn);
        }
    }

    bondgroup
}
