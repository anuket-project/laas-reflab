use serde::{Deserialize, Serialize};
use std::net::IpAddr;

use crate::prelude::{
    InventoryError, ModifiedFields, Switch, SwitchPort, SwitchReport, SwitchportReport,
};

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq, Clone)]
pub struct SwitchYaml {
    pub name: String,
    pub ip: IpAddr,
    pub username: String,
    pub password: String,
    #[serde(default)]
    pub switch_os: String,
    pub switchports: Vec<String>,
}

pub type SwitchDatabaseInfo = Option<(Switch, Vec<SwitchPort>)>;

impl SwitchYaml {
    pub fn generate_switch_report(
        &self,
        switch_database_info: SwitchDatabaseInfo,
    ) -> Result<SwitchReport, InventoryError> {
        if switch_database_info.is_none() {
            return Ok(SwitchReport::Created {
                switch_yaml: self.clone(),
            });
        }

        // Safe to unwrap
        let (db_switch, db_switchports) = switch_database_info.unwrap();
        let mut fields = ModifiedFields::default();

        if self.name != db_switch.name {
            fields.modified("name", self.name.clone(), db_switch.name.clone())?;
        }

        match db_switch.ip.parse::<IpAddr>() {
            Ok(db_ip) => {
                if self.ip != db_ip {
                    fields.modified("ip", self.ip.to_string(), db_switch.ip.clone())?;
                }
            }
            Err(e) => {
                return Err(InventoryError::AddrParse {
                    value: db_switch.ip.clone(),
                    source: e,
                });
            }
        }

        if self.username != db_switch.user {
            fields.modified("username", self.username.clone(), db_switch.user.clone())?;
        }

        if self.password != db_switch.pass {
            fields.modified("password", self.password.clone(), db_switch.pass.clone())?;
        }

        // TODO: Switch OS name comparison (not in MVP 1.1)
        let switchport_reports = self.generate_switchport_reports(&db_switch, &db_switchports)?;

        if fields.is_empty() {
            Ok(SwitchReport::Unchanged)
        } else {
            Ok(SwitchReport::Modified {
                switch_yaml: self.clone(),
                fields,
                switchport_reports,
            })
        }
    }

    /// Compare YAML-defined switchports against those in the database and
    /// produce a report for creations, removals, and unchanged entries.
    pub fn generate_switchport_reports(
        &self,
        db_switch: &Switch,
        db_switchports: &[SwitchPort],
    ) -> Result<Vec<SwitchportReport>, InventoryError> {
        let mut reports = Vec::new();

        // Created
        for yaml_name in &self.switchports {
            if !db_switchports.iter().any(|db_sp| &db_sp.name == yaml_name) {
                reports.push(SwitchportReport::new_created(
                    self.clone(),
                    yaml_name.clone(),
                ));
            }
        }

        // Removed
        for db_sp in db_switchports {
            if !self
                .switchports
                .iter()
                .any(|yaml_name| yaml_name == &db_sp.name)
            {
                // we have both db_switch and db_switchport available
                reports.push(SwitchportReport::new_removed(
                    db_sp.clone(),
                    db_switch.clone(),
                ));
            }
        }

        // Unchanged
        for _ in self
            .switchports
            .iter()
            .filter(|yaml_name| db_switchports.iter().any(|db_sp| &db_sp.name == *yaml_name))
        {
            reports.push(SwitchportReport::Unchanged {
                switch_yaml: self.clone(),
            });
        }

        Ok(reports)
    }
}
