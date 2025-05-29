use models::inventory::{Host, HostPort};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

use super::interface::InterfaceYaml;
use super::ipmi::IpmiYaml;

use crate::{
    prelude::{InventoryError, ModifiedFields, Report, fqdn_to_hostname_and_domain},
    utils::hostname_and_domain_to_fqdn,
};

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct HostYaml {
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
    pub(crate) fn report_diff(
        &self,
        host_info: Option<HostInfo>,
    ) -> Result<Report, InventoryError> {
        // We know the host is new if we don't have a host_info, this data will be expected to
        // be passed in during the host fetching process from the db
        if host_info.is_none() {
            return Ok(Report::new_created(self.server_name.clone()));
        }

        // unpack host info
        let (db_host, _db_ports, db_flavor_name) = host_info.unwrap();

        let mut modified_fields = ModifiedFields::new();

        // if we somehow got host info with a server_name that doesn't match the yaml we need to
        // return an error because we aren't even looking at the right host
        if self.server_name != db_host.server_name {
            return Err(InventoryError::HostNameMismatch {
                expected: self.server_name.clone(),
                actual: db_host.server_name.clone(),
            });
        }

        // TODO: Tracing + Docs

        // domain and hostname

        let (hostname, domain) = fqdn_to_hostname_and_domain(&db_host.fqdn);
        let yaml_fqdn = hostname_and_domain_to_fqdn(&self.server_name, &self.domain);

        if self.domain != domain || self.server_name != hostname {
            modified_fields.modified("fqdn", &db_host.fqdn, yaml_fqdn)?;
        }

        // flavor

        if db_flavor_name != self.flavor_name {
            modified_fields.modified(
                "flavor",
                db_flavor_name.to_string(),
                self.flavor_name.clone(),
            )?;
        }

        // iol_id

        if db_host.iol_id != self.iol_id {
            modified_fields.modified("iol_id", db_host.iol_id.clone(), self.iol_id.clone())?;
        }

        // serial_number

        if db_host.serial != self.serial_number {
            modified_fields.modified(
                "serial",
                db_host.serial.clone(),
                self.serial_number.clone(),
            )?;
        }

        // TODO: model_number

        if db_host.projects.len() != 1 {
            return Err(InventoryError::TooManyProjects(db_host.projects.clone()));
        }

        if db_host.projects[0] != self.project {
            modified_fields.modified(
                "project",
                db_host.projects[0].clone(),
                self.project.clone(),
            )?;
        }

        // TODO: ipmi
        if let Some(ipmi) = self.ipmi_yaml.report_diff(&db_host)? {
            modified_fields.merge("ipmi", ipmi)?;
        }

        // TODO: interfaces

        // return valid
        if modified_fields.is_empty() {
            Ok(Report::new_unchanged(self.server_name.clone()))
        } else {
            Ok(Report::new_modified(
                self.server_name.clone(),
                modified_fields,
            ))
        }
    }

    pub async fn update_host_record(
        &self,
        _db_host: &Host,
        pool: &PgPool,
    ) -> Result<(), InventoryError> {
        sqlx::query!(
            r#"
    UPDATE hosts
    SET
      fqdn        = $2,
      flavor      = (SELECT id FROM flavors WHERE name = $3),
      iol_id      = $4,
      serial      = $5,
      ipmi_fqdn   = $6,
      ipmi_mac    = $7,
      ipmi_user   = $8,
      ipmi_pass   = $9,
      projects    = $10
    WHERE server_name = $1;
    "#,
            self.server_name,                                // $1: VARCHAR → String
            format!("{}.{}", self.server_name, self.domain), // $2: VARCHAR → String
            self.flavor_name,                                // $3: VARCHAR → String
            self.iol_id,                                     // $4: VARCHAR → String
            self.serial_number,                              // $5: VARCHAR → String
            format!("{}.{}", self.ipmi_yaml.hostname, self.ipmi_yaml.domain), // $6: VARCHAR → String
            self.ipmi_yaml.mac,    // $7: macaddr  → String
            self.ipmi_yaml.user,   // $8: VARCHAR → String
            self.ipmi_yaml.pass,   // $9: VARCHAR → String
            json!([self.project]), // $10: JSONB []   → String
        )
        .execute(pool)
        .await
        .map_err(|e| InventoryError::Sqlx {
            context: "While updating host record".into(),
            source: e,
        })?;

        // TODO: HostPorts/Interfaces
        // sqlx::query!(
        //     r#"DELETE FROM host_ports WHERE on_host = $1;"#,
        //     db_host.id.into_id().into_uuid()
        // )
        // .execute(pool)
        // .await
        // .map_err(|e| InventoryError::Sqlx {
        //     context: "While deleting host ports".to_string(),
        //     source: e,
        // })?;

        Ok(())
    }

    /// Insert a brand-new host row into `hosts`.
    pub async fn create_host_record(&self, pool: &PgPool) -> Result<(), InventoryError> {
        let id = Uuid::new_v4();

        sqlx::query!(
            r#"
            INSERT INTO hosts (
              id,
              server_name,
              fqdn,
              flavor,
              iol_id,
              serial,
              ipmi_fqdn,
              ipmi_mac,
              ipmi_user,
              ipmi_pass,
              projects
            ) VALUES (
              $1, $2, $3,
              (SELECT id FROM flavors WHERE name = $4),
              $5, $6, $7, $8, $9, $10, $11
            )
            "#,
            id,                                                               // $1: UUID
            self.server_name,                                                 // $2: VARCHAR
            format!("{}.{}", self.server_name, self.domain),                  // $3: VARCHAR
            self.flavor_name,   // $4: VARCHAR → flavor fk
            self.iol_id,        // $5: VARCHAR
            self.serial_number, // $6: VARCHAR
            format!("{}.{}", self.ipmi_yaml.hostname, self.ipmi_yaml.domain), // $7: VARCHAR
            self.ipmi_yaml.mac, // $8: MACADDR
            self.ipmi_yaml.user, // $9: VARCHAR
            self.ipmi_yaml.pass, // $10: VARCHAR
            json!([self.project]), // $11: [JSONB]
        )
        .execute(pool)
        .await
        .map_err(|e| InventoryError::Sqlx {
            context: "While inserting new host".into(),
            source: e,
        })?;

        Ok(())
    }
}
