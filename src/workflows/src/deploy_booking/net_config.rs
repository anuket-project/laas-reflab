//! Copyright (c) 2023 University of New Hampshire
//! SPDX-License-Identifier: MIT

use common::prelude::tracing::{self, info};

use models::{
    dal::{DBTable, EasyTransaction, ExistingRow, FKey},
    dashboard::{Aggregate, Instance},
    inventory::{Host, HostPort},
};

use std::collections::*;
use tascii::{prelude::*, task_trait::AsyncRunnable};

use crate::resource_management::network::{
    BondGroup, NetworkConfig, NetworkConfigBuilder, VlanConnection,
};

pub async fn mgmt_network_config(
    host_id: FKey<Host>,
    t: &mut EasyTransaction<'_>,
) -> NetworkConfig {
    // set each iface to 98 + 99, no bond groups
    let host = host_id
        .get(t)
        .await
        .expect("host did not exist by given fk?");
    let mut builder = NetworkConfigBuilder::new();
    for port in host.ports(t).await.expect("didn't get ports?") {
        builder = builder.bond(
            BondGroup::new()
                .with_vlan(VlanConnection::from_pair(t, 99, true).await)
                .with_vlan(VlanConnection::from_pair(t, 98, false).await)
                .with_port(port.id),
        );
    }

    let v = builder.persist(false).build();

    info!("built a network config for the host: {v:#?}");

    v
}

pub async fn postprovision_network_config(
    host_id: FKey<Host>,
    aggregate_id: FKey<Aggregate>,
    t: &mut EasyTransaction<'_>,
) -> NetworkConfig {
    let networks = aggregate_id
        .get(t)
        .await
        .unwrap()
        .vlans
        .get(t)
        .await
        .unwrap()
        .into_inner();

    let mut public_vlan_id = None;

    for (net, vlan) in networks.networks {
        let net = net.get(t).await.unwrap();
        let vlan = vlan.get(t).await.unwrap();

        if net.public {
            public_vlan_id = Some(vlan.vlan_id as u16);
            break;
        }
    }

    let public_vlan_id = public_vlan_id.expect("pod contained no public networks");

    let host = host_id
        .get(t)
        .await
        .expect("host did not exist by given fk?");
    let mut builder = NetworkConfigBuilder::new();
    for port in host.ports(t).await.expect("didn't get ports?") {
        builder = builder.bond(
            BondGroup::new()
                .with_vlan(VlanConnection::from_pair(t, 99, true).await)
                .with_vlan(VlanConnection::from_pair(t, public_vlan_id as i16, false).await)
                .with_port(port.id),
        );
    }

    let v = builder.persist(false).build();

    info!("built a network config for the host: {v:#?}");

    v
}

pub async fn empty_network_config(
    host_id: FKey<Host>,
    t: &mut EasyTransaction<'_>,
) -> NetworkConfig {
    let host = host_id.get(t).await.expect("host did not give a fk?");
    let mut builder = NetworkConfigBuilder::new();
    for port in host.ports(t).await.expect("didn't get ports?") {
        builder = builder.bond(
            BondGroup::new()
                .with_vlan(VlanConnection::from_pair(t, 99, true).await)
                .with_port(port.id),
        );
    }

    let v = builder.persist(true).build();

    info!("built a network config for the host: {v:#?}");

    v
}

pub async fn prod_network_config(
    host_id: FKey<Host>,
    deployed_as: FKey<models::dashboard::Instance>,
    t: &mut EasyTransaction<'_>,
) -> NetworkConfig {
    async fn bg_config_to_bg(
        t: &mut EasyTransaction<'_>,
        bgc: &models::dashboard::BondGroupConfig,
        interfaces: &HashMap<String, HostPort>,
        networks: &models::dashboard::NetworkAssignmentMap,
    ) -> BondGroup {
        let mut bg = BondGroup::new();

        info!("Translating bg_config to bg. Bgc is {bgc:#?} while interfaces is {interfaces:#?} and networks is {networks:#?}");

        for port in bgc.member_interfaces.iter() {
            bg = bg.with_port(interfaces.get(port).unwrap().id);
        }

        for vcc in bgc.connects_to.iter() {
            let vlan = *networks.networks.get(&vcc.network).unwrap();
            let vconn = VlanConnection {
                vlan,
                tagged: vcc.tagged,
            };

            bg = bg.with_vlan(vconn);
        }

        // Make sure to keep the 99/ipmi connection on all bondgroups/ports
        bg = bg.with_vlan(VlanConnection {
            vlan: models::inventory::Vlan::select()
                .where_field("vlan_id")
                .equals(99_i16)
                .run(t)
                .await
                .expect("need at least the ipmi vlan, hardcode requirement")
                .first()
                .unwrap()
                .id,
            tagged: true,
        });

        bg
    }

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

        let bg = bg_config_to_bg(t, bg_config, &ports_by_name, &network_assignments).await;

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

async fn bg_config_to_bg(
    t: &mut EasyTransaction<'_>,
    bgc: &models::dashboard::BondGroupConfig,
    interfaces: &HashMap<String, HostPort>,
    networks: &models::dashboard::NetworkAssignmentMap,
) -> BondGroup {
    let mut bg = BondGroup::new();

    tracing::info!("Translating bg_config to bg. Bgc is {bgc:#?} while interfaces is {interfaces:#?} and networks is {networks:#?}");

    for port in bgc.member_interfaces.iter() {
        bg = bg.with_port(interfaces.get(port).unwrap().id);
    }

    for vcc in bgc.connects_to.iter() {
        let vlan = *networks.networks.get(&vcc.network).unwrap();
        let vconn = VlanConnection {
            vlan,
            tagged: vcc.tagged,
        };

        bg = bg.with_vlan(vconn);
    }

    // Make sure to keep the 99/ipmi connection on all bondgroups/ports
    bg = bg.with_vlan(VlanConnection {
        vlan: models::inventory::Vlan::select()
            .where_field("vlan_id")
            .equals(99 as i16)
            .run(t)
            .await
            .expect("need at least the ipmi vlan, hardcode requirement")
            .get(0)
            .unwrap()
            .id,
        tagged: true,
    });

    bg
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

        let bg = bg_config_to_bg(t, bg_config, &ports_by_name, &network_assignments).await
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
