use super::super::types::{BondGroup, NetworkConfig, NetworkConfigBuilder, VlanConnection};
use dal::{EasyTransaction, FKey};
use models::inventory::Host;
use tracing::info;

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
