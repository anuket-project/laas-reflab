use crate::prelude::{InventoryError, InventoryYaml};

use glob::glob;
use std::path::Path;

/// Loads all YAML inventory files under the given directory and
/// returns a single `InventoryYaml` containing all switches and hosts.
pub(crate) fn load_inventory(dir: &Path) -> Result<InventoryYaml, Vec<InventoryError>> {
    let mut all_switches = Vec::new();
    let mut all_hosts = Vec::new();
    let mut errors = Vec::new();

    if !dir.exists() {
        errors.push(InventoryError::IoPath {
            path: dir.to_path_buf(),
            message: "Path does not exist".into(),
        });
        return Err(errors);
    }
    if !dir.is_dir() {
        errors.push(InventoryError::IoPath {
            path: dir.to_path_buf(),
            message: "Path is not a directory".into(),
        });
        return Err(errors);
    }

    let pattern = format!("{}/**/*.yaml", dir.display());
    let entries = match glob(&pattern) {
        Ok(paths) => paths,
        Err(e) => {
            errors.push(InventoryError::Pattern(e));
            return Err(errors);
        }
    };

    for entry in entries {
        match entry {
            Ok(path) => {
                let path_str = path.display().to_string();

                // Read file contents
                let data = match std::fs::read_to_string(&path) {
                    Ok(s) => s,
                    Err(e) => {
                        errors.push(InventoryError::Io {
                            path: path_str.clone(),
                            source: e,
                        });
                        continue;
                    }
                };

                match serde_yaml::from_str::<InventoryYaml>(&data) {
                    Ok(inv) => {
                        all_switches.extend(inv.switches);
                        all_hosts.extend(inv.hosts);
                    }
                    Err(e) => errors.push(InventoryError::Yaml {
                        path: path_str,
                        source: e,
                    }),
                }
            }
            Err(e) => {
                errors.push(InventoryError::Glob(e));
            }
        }
    }

    if errors.is_empty() {
        Ok(InventoryYaml {
            switches: all_switches,
            hosts: all_hosts,
        })
    } else {
        Err(errors)
    }
}
