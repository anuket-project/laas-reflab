// pub async fn postprovision_network_config(
//     host_id: FKey<Host>,
//     aggregate_id: FKey<Aggregate>,
//     t: &mut EasyTransaction<'_>,
// ) -> NetworkConfig {
//     let networks = aggregate_id
//         .get(t)
//         .await
//         .unwrap()
//         .vlans
//         .get(t)
//         .await
//         .unwrap()
//         .into_inner();
//
//     let mut public_vlan_id = None;
//
//     for (net, vlan) in networks.networks {
//         let net = net.get(t).await.unwrap();
//         let vlan = vlan.get(t).await.unwrap();
//
//         if net.public {
//             public_vlan_id = Some(vlan.vlan_id as u16);
//             break;
//         }
//     }
//
//     let public_vlan_id = public_vlan_id.expect("pod contained no public networks");
//
//     let host = host_id
//         .get(t)
//         .await
//         .expect("host did not exist by given fk?");
//     let mut builder = NetworkConfigBuilder::new();
//     for port in host.ports(t).await.expect("didn't get ports?") {
//         builder = builder.bond(
//             BondGroup::new()
//                 .with_vlan(VlanConnection::from_pair(t, 99, true).await)
//                 .with_vlan(VlanConnection::from_pair(t, public_vlan_id as i16, false).await)
//                 .with_port(port.id),
//         );
//     }
//
//     let v = builder.persist(false).build();
//
//     info!("built a network config for the host: {v:#?}");
//
//     v
// }
