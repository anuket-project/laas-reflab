use models::inventory::{Host, HostPort};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::prelude::{
    HostReport, InterfaceYaml, InventoryError, IpmiYaml, ModifiedFields, Reportable, SwitchPort,
    fqdn_to_hostname_and_domain, generate_created_interface_reports, generate_interface_reports,
    hostname_and_domain_to_fqdn,
};

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq, Clone)]
pub struct HostYaml {
    pub server_name: String,
    pub domain: String,
    #[serde(rename = "flavor")]
    pub flavor_name: String,
    pub iol_id: String,
    pub serial_number: String,
    pub model_number: Option<String>,
    pub project: String,
    #[serde(rename = "ipmi")]
    pub ipmi_yaml: IpmiYaml,
    pub interfaces: Vec<InterfaceYaml>,
}

pub(crate) type HostInfo = (Host, Vec<HostPort>, String);

impl HostYaml {
    pub(crate) async fn generate_host_report(
        &self,
        host_info: Option<HostInfo>,
        switchport_map: &HashMap<String, Vec<SwitchPort>>,
    ) -> Result<HostReport, InventoryError> {
        // new host case
        if host_info.is_none() {
            let iface_reports = generate_created_interface_reports(
                &self.server_name,
                &self.flavor_name,
                &self.interfaces,
            );
            return Ok(HostReport::new_created(self.clone(), iface_reports));
        }

        // if some db info exists we are in Unchanged or Modified case
        let (db_host, db_ports, db_flavor) = host_info.unwrap();

        // make sure names match
        if db_host.server_name != self.server_name {
            return Err(InventoryError::HostNameMismatch {
                expected: self.server_name.clone(),
                actual: db_host.server_name,
            });
        }

        let mut changes = ModifiedFields::new();

        // fqdn
        let (cur_name, cur_domain) = fqdn_to_hostname_and_domain(&db_host.fqdn);

        let desired = hostname_and_domain_to_fqdn(&self.server_name, &self.domain);
        if cur_name != self.server_name || cur_domain != self.domain {
            changes.modified("fqdn", &db_host.fqdn, &desired)?;
        }

        // flavor
        if db_flavor != self.flavor_name {
            changes.modified("flavor", &db_flavor, &self.flavor_name)?;
        }

        // iol_id
        if db_host.iol_id != self.iol_id {
            changes.modified("iol_id", &db_host.iol_id, &self.iol_id)?;
        }

        // serial_number
        if db_host.serial != self.serial_number {
            changes.modified("serial", &db_host.serial, &self.serial_number)?;
        }

        // project
        if db_host.projects.len() != 1 {
            return Err(InventoryError::TooManyProjects(db_host.projects.clone()));
        }
        if db_host.projects[0] != self.project {
            changes.modified("project", &db_host.projects[0], &self.project)?;
        }

        // ipmi
        if let Some(ipmi_diff) = self.ipmi_yaml.report_diff(&db_host)? {
            changes.merge("ipmi", ipmi_diff)?;
        }

        let iface_reports = generate_interface_reports(
            &self.server_name,
            &self.flavor_name,
            &self.interfaces,
            &db_ports,
            switchport_map,
        )
        .await?;

        // TODO: model number comparison (not in MVP 1.1)

        if changes.is_empty() && iface_reports.iter().all(|r| r.is_unchanged()) {
            Ok(HostReport::new_unchanged(self.clone()))
        } else {
            Ok(HostReport::new_modified(
                self.clone(),
                changes,
                iface_reports,
            ))
        }
    }
}
