use dal::{new_client, AsEasyTransaction, DBTable, EasyTransaction, FKey};

use models::{
    dashboard::{BondGroupConfig, NetworkAssignmentMap},
    inventory::Vlan,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Hash, Serialize, Deserialize)]
pub struct VlanConnection {
    pub vlan: FKey<Vlan>,
    pub tagged: bool,
}

impl VlanConnection {
    pub async fn from_pair(t: &mut EasyTransaction<'_>, vlan_id: i16, tagged: bool) -> Self {
        Self {
            vlan: Self::fetch_vlan_id(vlan_id, t).await,
            tagged,
        }
    }
    pub async fn fetch_vlan_id(vlan_id: i16, t: &mut EasyTransaction<'_>) -> FKey<Vlan> {
        Vlan::select()
            .where_field("vlan_id")
            .equals(vlan_id)
            .run(t)
            .await
            .expect("Missing VLAN in database")
            .first()
            .unwrap()
            .id
    }
}

/// In memory representation of a connection intended to be managed through NetworkManager
///
/// device_name - ens1f0 / eno40 / etc
///
/// network_name - public1 / private2 / etc
///
/// vlan_id - 109 / 250 / etc
///
/// tagged true / false
///
/// connection_number 1 / 2 /etc (only needed to create distinct interface names)

#[derive(Debug, Clone)]
pub struct NetworkManagerVlanConnection {
    pub device_name: String,
    pub network_name: String,
    pub vlan_id: i16,
    pub tagged: bool,
    pub connection_number: usize,
}

impl NetworkManagerVlanConnection {
    /// Renders a string representing a single network configuration for kickstart.
    ///
    /// Example: network --device=ens4f0 --vlanid=109 --interfacename pub1v109 --activate
    ///
    /// https://pykickstart.readthedocs.io/en/latest/kickstart-docs.html#network
    pub fn render_kickstart_network_config(&self) -> String {
        let network_name = &self.network_name;
        let vlan_id = self.vlan_id;
        let connection_number = self.connection_number;
        let device_name = &self.device_name;

        let interface_name = format!("{network_name:.3}{connection_number}v{vlan_id}");
        format!("network --device={device_name} --vlanid={vlan_id} --interfacename={interface_name} --activate")
    }

    /// Example : nmcli con add type vlan connection.id tagged-public-ens4f0.118 ifname pub1v118 vlan.id 118 dev ens4f0
    pub fn render_nmcli_add_command(self) -> String {
        let network_name = self.network_name;
        let vlan_id = self.vlan_id;
        let connection_number = self.connection_number;
        let device_name = self.device_name;
        let tagged = if self.tagged { "tagged" } else { "untagged" };

        let interface_name = format!("{network_name:.3}{connection_number}v{vlan_id}");
        format!("nmcli con add type vlan connection.id {tagged}-{network_name}-{device_name}.{vlan_id} ifname {interface_name} vlan.id {vlan_id} dev {device_name}")
    }
}

/// Uses information stored in a NetworkAssignmentMap and a list of BondGroupConfig to construct a NetworkManagerVlanConnection for each configured host connection.
pub async fn create_network_manager_vlan_connections_from_bondgroups(
    network_assignment_map: &NetworkAssignmentMap,
    bondgroups: &[BondGroupConfig],
) -> Result<Vec<NetworkManagerVlanConnection>, anyhow::Error> {
    let mut client = new_client().await.unwrap();
    let mut transaction = client.easy_transaction().await.unwrap();

    let mut collected_nm_vlan_connections: Vec<NetworkManagerVlanConnection> = vec![];
    for (connection_number, bondgroup) in bondgroups.iter().enumerate() {
        for connection in bondgroup.connects_to.iter() {
            let network_id = connection.network;
            let is_tagged = connection.tagged;

            for interface in &bondgroup.member_interfaces {
                let vlan_id = network_assignment_map
                    .networks
                    .get(&network_id)
                    .unwrap()
                    .get(&mut transaction)
                    .await?
                    .vlan_id;
                let network_name = &network_id.get(&mut transaction).await?.name;
                collected_nm_vlan_connections.push(NetworkManagerVlanConnection {
                    device_name: interface.to_string(),
                    vlan_id,
                    tagged: is_tagged,
                    network_name: network_name.to_string(),
                    connection_number,
                });
            }
        }
    }

    Ok(collected_nm_vlan_connections)
}
