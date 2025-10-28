use models::inventory::{Arch, Flavor, StorageType};
use serde::{Deserialize, Serialize};

use crate::prelude::{FlavorReport, InventoryError, ModifiedFields};

fn format_bytes(bytes: i64) -> String {
    const KB: i64 = 1024;
    const MB: i64 = KB * 1024;
    const GB: i64 = MB * 1024;
    const TB: i64 = GB * 1024;

    if bytes >= TB {
        format!("{} TB", bytes / TB)
    } else if bytes >= GB {
        format!("{} GB", bytes / GB)
    } else if bytes >= MB {
        format!("{} MB", bytes / MB)
    } else if bytes >= KB {
        format!("{} KB", bytes / KB)
    } else {
        format!("{} B", bytes)
    }
}

fn format_optional_old(opt: &Option<String>) -> String {
    match opt {
        Some(s) if !s.is_empty() => s.clone(),
        Some(_) => "\"\"".to_string(),
        None => "None".to_string(),
    }
}

fn format_optional_new(opt: &Option<String>) -> String {
    match opt {
        Some(s) if !s.is_empty() => s.clone(),
        _ => "?".to_string(),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FlavorYaml {
    pub name: String,
    pub description: Option<String>,
    pub brand: Option<String>,
    pub model: Option<String>,
    pub arch: Arch,
    pub cpu_count: Option<i32>,
    pub cpu_frequency_mhz: Option<i32>,
    pub cpu_model: Option<String>,
    pub ram_bytes: Option<i64>,
    pub root_size_bytes: Option<i64>,
    pub disk_size_bytes: Option<i64>,
    pub storage_type: Option<StorageType>,
    pub network_speed_mbps: Option<i32>,
    pub network_interfaces: Option<i32>,
}

impl FlavorYaml {
    pub(crate) fn generate_flavor_report(
        &self,
        db_flavor: Option<Flavor>,
    ) -> Result<FlavorReport, InventoryError> {
        let Some(db_flavor) = db_flavor else {
            return Ok(FlavorReport::new_created(self.clone()));
        };

        // check if names match
        if db_flavor.name != self.name {
            return Err(InventoryError::NotFound(format!(
                "Flavor name mismatch: expected '{}', got '{}'",
                self.name, db_flavor.name
            )));
        }

        let mut changes = ModifiedFields::new();

        // description?
        let old_desc = format_optional_old(&db_flavor.description);
        let new_desc = format_optional_new(&self.description);
        if old_desc != new_desc {
            changes.modified("description?", &old_desc, &new_desc)?;
        }

        // brand?
        let old_brand = format_optional_old(&db_flavor.brand);
        let new_brand = format_optional_new(&self.brand);
        if old_brand != new_brand {
            changes.modified("brand?", &old_brand, &new_brand)?;
        }

        // model?
        let old_model_field = format_optional_old(&db_flavor.model);
        let new_model_field = format_optional_new(&self.model);
        if old_model_field != new_model_field {
            changes.modified("model?", &old_model_field, &new_model_field)?;
        }

        // arch (required)
        if db_flavor.arch != self.arch {
            changes.modified("arch", db_flavor.arch.to_string(), self.arch.to_string())?;
        }

        // cpu_count?
        if db_flavor.cpu_count != self.cpu_count {
            changes.modified(
                "cpu_count?",
                db_flavor
                    .cpu_count
                    .map_or("None".to_string(), |v| v.to_string()),
                self.cpu_count.map_or("?".to_string(), |v| v.to_string()),
            )?;
        }

        // cpu_frequency_mhz?
        if db_flavor.cpu_frequency_mhz != self.cpu_frequency_mhz {
            changes.modified(
                "cpu_frequency_mhz?",
                db_flavor
                    .cpu_frequency_mhz
                    .map_or("None".to_string(), |v| v.to_string()),
                self
                    .cpu_frequency_mhz
                    .map_or("?".to_string(), |v| v.to_string()),
            )?;
        }

        // cpu_model?
        let old_cpu_model = format_optional_old(&db_flavor.cpu_model);
        let new_cpu_model = format_optional_new(&self.cpu_model);
        if old_cpu_model != new_cpu_model {
            changes.modified("cpu_model?", &old_cpu_model, &new_cpu_model)?;
        }

        // ram_bytes?
        if db_flavor.ram_bytes != self.ram_bytes {
            changes.modified(
                "ram_bytes?",
                db_flavor
                    .ram_bytes
                    .map_or("None".to_string(), format_bytes),
                self.ram_bytes.map_or("?".to_string(), format_bytes),
            )?;
        }

        // root_size_bytes?
        if db_flavor.root_size_bytes != self.root_size_bytes {
            changes.modified(
                "root_size_bytes?",
                db_flavor
                    .root_size_bytes
                    .map_or("None".to_string(), format_bytes),
                self
                    .root_size_bytes
                    .map_or("?".to_string(), format_bytes),
            )?;
        }

        // disk_size_bytes?
        if db_flavor.disk_size_bytes != self.disk_size_bytes {
            changes.modified(
                "disk_size_bytes?",
                db_flavor
                    .disk_size_bytes
                    .map_or("None".to_string(), format_bytes),
                self
                    .disk_size_bytes
                    .map_or("?".to_string(), format_bytes),
            )?;
        }

        // storage_type?
        if db_flavor.storage_type != self.storage_type {
            let old_val = db_flavor
                .storage_type
                .map(|st| st.to_string())
                .unwrap_or_else(|| "None".to_string());
            let new_val = self
                .storage_type
                .map(|st| st.to_string())
                .unwrap_or_else(|| "?".to_string());
            if old_val != new_val {
                changes.modified("storage_type?", &old_val, &new_val)?;
            }
        }

        // network_speed_mbps?
        if db_flavor.network_speed_mbps != self.network_speed_mbps {
            changes.modified(
                "network_speed_mbps?",
                db_flavor
                    .network_speed_mbps
                    .map_or("None".to_string(), |v| v.to_string()),
                self
                    .network_speed_mbps
                    .map_or("?".to_string(), |v| v.to_string()),
            )?;
        }

        // network_interfaces?
        if db_flavor.network_interfaces != self.network_interfaces {
            changes.modified(
                "network_interfaces?",
                db_flavor
                    .network_interfaces
                    .map_or("None".to_string(), |v| v.to_string()),
                self
                    .network_interfaces
                    .map_or("?".to_string(), |v| v.to_string()),
            )?;
        }

        if changes.is_empty() {
            Ok(FlavorReport::new_unchanged(self.name.clone()))
        } else {
            Ok(FlavorReport::new_modified(self.clone(), changes))
        }
    }
}
