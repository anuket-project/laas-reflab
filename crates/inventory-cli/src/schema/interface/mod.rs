use mac_address::MacAddress;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

use crate::prelude::{
    HostPort, InterfaceReport, InventoryError, ModifiedFields, Reportable, SwitchPort,
};

mod connection;
pub use connection::ConnectionYaml;

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq, Clone)]
pub struct InterfaceYaml {
    pub name: String,
    pub mac: MacAddress,
    pub bus_addr: String,
    pub management_vlan_id: Option<i16>,
    pub bmc_vlan_id: Option<i16>,
    pub connection: ConnectionYaml,
}

/// Compare desired YAML interfaces vs actual DB hostports + switchports,
/// emitting a flat Vec<InterfaceReport>.
pub async fn generate_interface_reports(
    server_name: &str,
    flavor_name: &str,
    yaml_ifaces: &[InterfaceYaml],
    db_ports: &[HostPort],
    _switch_switchport_map: &HashMap<String, Vec<SwitchPort>>,
) -> Result<Vec<InterfaceReport>, InventoryError> {
    let yaml_names: HashSet<&String> = yaml_ifaces.iter().map(|i| &i.name).collect();
    let db_names: HashSet<&String> = db_ports.iter().map(|p| &p.name).collect();

    let mut reports = Vec::with_capacity(yaml_ifaces.len() + db_ports.len());

    for yf in yaml_ifaces.iter() {
        match db_names.contains(&yf.name) {
            false => {
                // created case
                reports.push(InterfaceReport::new_created(
                    server_name.to_string(),
                    flavor_name.to_string(),
                    yf.clone(),
                ));
            }
            true => {
                // modified or unchanged case
                let dbp = db_ports
                    .iter()
                    .find(|p| p.name == yf.name)
                    .ok_or_else(|| InventoryError::NotFound(format!("hostport {}", yf.name)))?;

                let mut fields = ModifiedFields::new();

                if dbp.mac.to_string() != yf.mac.to_string() {
                    fields
                        .modified("mac", dbp.mac.to_string(), yf.mac.to_string())
                        .ok();
                }
                if dbp.bus_addr != yf.bus_addr {
                    fields
                        .modified("bus_addr", dbp.bus_addr.clone(), yf.bus_addr.clone())
                        .ok();
                }
                if dbp.management_vlan_id != yf.management_vlan_id {
                    fields
                        .modified(
                            "management_vlan_id",
                            format!("{:?}", dbp.management_vlan_id),
                            format!("{:?}", yf.management_vlan_id),
                        )
                        .ok();
                }

                if dbp.bmc_vlan_id != yf.bmc_vlan_id {
                    fields
                        .modified(
                            "bmc_vlan_id",
                            format!("{:?}", dbp.bmc_vlan_id),
                            format!("{:?}", yf.bmc_vlan_id),
                        )
                        .ok();
                }

                if fields.is_empty() {
                    reports.push(InterfaceReport::new_unchanged());
                } else {
                    reports.push(InterfaceReport::new_modified(
                        server_name.to_string(),
                        flavor_name.to_string(),
                        fields,
                        yf.clone(),
                    ));
                }
            }
        }
    }

    // removed case
    for dbp in db_ports.iter().filter(|p| !yaml_names.contains(&p.name)) {
        reports.push(InterfaceReport::new_removed(
            server_name.to_string(),
            dbp.clone(),
        ));
    }

    reports.sort_by_key(|r| r.sort_order());
    Ok(reports)
}

pub fn generate_created_interface_reports(
    server_name: &str,
    flavor_name: &str,
    yaml_ifaces: &[InterfaceYaml],
) -> Vec<InterfaceReport> {
    yaml_ifaces
        .iter()
        .map(|yf| {
            InterfaceReport::new_created(
                server_name.to_string(),
                flavor_name.to_string(),
                yf.clone(),
            )
        })
        .collect()
}
