use crate::prelude::{HostYaml, InventoryError, InventoryYaml};

use glob::glob;
use std::path::Path;

pub(crate) fn load_inventory_hosts(dir: &Path) -> Result<Vec<HostYaml>, Vec<InventoryError>> {
    let mut hosts = Vec::new();
    let mut errors = Vec::new();

    // path checks
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

    // construct glob pattern from dir path argument
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

                // read file
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

                // parse into yaml
                match serde_yaml::from_str::<InventoryYaml>(&data) {
                    Ok(inv) => hosts.push(inv.host),
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
        Ok(hosts)
    } else {
        Err(errors)
    }
}
