use mac_address::MacAddress;
use serde::{Deserialize, Serialize};

use crate::prelude::{
    Host, InventoryError, ModifiedFields, fqdn_to_hostname_and_domain, hostname_and_domain_to_fqdn,
};

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct IpmiYaml {
    pub hostname: String,
    pub ip4: String,
    pub mac: MacAddress,
    pub user: String,
    pub pass: String,
    pub domain: String,
}

impl IpmiYaml {
    pub(crate) fn report_diff(
        &self,
        db_host: &Host,
    ) -> Result<Option<ModifiedFields>, InventoryError> {
        let mut mf = ModifiedFields::new();

        // TODO: ip4 doesn't exist on host

        // ipmi_fqdn
        let (hostname, domain) = fqdn_to_hostname_and_domain(&db_host.ipmi_fqdn);
        let yaml_fqdn = hostname_and_domain_to_fqdn(&self.hostname, &self.domain);

        if self.domain != domain || self.hostname != hostname {
            mf.modified("fqdn", &db_host.ipmi_fqdn, yaml_fqdn)?;
        }

        // mac
        let db_mac = MacAddress::new(db_host.ipmi_mac.to_array());
        if self.mac != db_mac {
            mf.modified("mac", db_mac.to_string(), self.mac.to_string())?;
        }

        // user
        if self.user != db_host.ipmi_user {
            mf.modified("user", &db_host.ipmi_user, &self.user)?;
        }

        // pass
        if self.pass != db_host.ipmi_pass {
            mf.modified("pass", &db_host.ipmi_pass, &self.pass)?;
        }

        Ok(if mf.is_empty() { None } else { Some(mf) })
    }
}
