use models::inventory::Lab;
use serde::{Deserialize, Serialize};

use crate::prelude::{InventoryError, LabReport, ModifiedFields};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LabYaml {
    pub name: String,
    pub location: String,
    pub email: String,
    pub phone: String,
    pub is_dynamic: bool,
}

impl LabYaml {
    pub(crate) fn generate_lab_report(
        &self,
        db_lab: Option<Lab>,
    ) -> Result<LabReport, InventoryError> {
        let Some(db_lab) = db_lab else {
            return Ok(LabReport::new_created(self.clone()));
        };

        // check if names match
        if db_lab.name != self.name {
            return Err(InventoryError::NotFound(format!(
                "Lab name mismatch: expected '{}', got '{}'",
                self.name, db_lab.name
            )));
        }

        let mut changes = ModifiedFields::new();

        // location
        if db_lab.location != self.location {
            changes.modified("location", &db_lab.location, &self.location)?;
        }

        // email
        if db_lab.email != self.email {
            changes.modified("email", &db_lab.email, &self.email)?;
        }

        // phone
        if db_lab.phone != self.phone {
            changes.modified("phone", &db_lab.phone, &self.phone)?;
        }

        // is_dynamic
        if db_lab.is_dynamic != self.is_dynamic {
            changes.modified(
                "is_dynamic",
                db_lab.is_dynamic.to_string(),
                self.is_dynamic.to_string(),
            )?;
        }

        if changes.is_empty() {
            Ok(LabReport::new_unchanged(self.name.clone()))
        } else {
            Ok(LabReport::new_modified(self.clone(), changes))
        }
    }
}
