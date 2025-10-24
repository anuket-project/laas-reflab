use http::Uri;
use models::dashboard::uri_vec_serde;
use models::{
    dashboard::ImageKernelArg,
    dashboard::image::{Distro, Image},
    inventory::Arch,
};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

use crate::prelude::{ImageReport, InventoryError, KernelArgReport, ModifiedFields, Reportable};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ImageYaml {
    pub name: String,
    pub cobbler_name: String,
    pub flavors: Vec<String>,
    pub distro: Distro,
    pub arch: Arch,
    pub version: String,
    #[serde(with = "http_serde::uri")]
    pub http_unattended_install_config_path: Uri,
    #[serde(with = "http_serde::uri")]
    pub http_iso_path: Uri,
    #[serde(with = "http_serde::uri")]
    pub tftp_kernel_path: Uri,
    #[serde(with = "uri_vec_serde")]
    pub tftp_initrd_paths: Vec<Uri>,
    pub kernel_args: Vec<KernelArg>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum KernelArg {
    KeyValue { key: String, value: String },
    Flag(String),
}

impl KernelArg {
    pub fn generate_created_kernel_arg_reports(
        image_name: &str,
        kernel_args: &Vec<KernelArg>,
    ) -> Vec<KernelArgReport> {
        kernel_args
            .iter()
            .map(|k_arg| KernelArgReport::new_created(image_name.to_string(), k_arg.clone()))
            .collect()
    }

    pub fn generate_kernel_arg_reports(
        image_name: &str,
        yaml_kernel_args: &Vec<KernelArg>,
        db_kernel_args: &Vec<ImageKernelArg>,
    ) -> Vec<KernelArgReport> {
        let mut reports = Vec::new();

        // Convert YAML kernel args to comparable strings
        let yaml_args: HashSet<String> = yaml_kernel_args
            .iter()
            .map(|arg| match arg {
                KernelArg::Flag(flag) => flag.clone(),
                KernelArg::KeyValue { key, value } => format!("{}={}", key, value),
            })
            .collect();

        // Convert DB kernel args to comparable strings
        let db_args: HashSet<String> = db_kernel_args
            .iter()
            .map(|arg| arg.render_to_kernel_arg())
            .collect();

        // Find created kernel args (in YAML but not in DB)
        for yaml_arg in yaml_kernel_args {
            let yaml_arg_str = match yaml_arg {
                KernelArg::Flag(flag) => flag.clone(),
                KernelArg::KeyValue { key, value } => format!("{}={}", key, value),
            };
            if !db_args.contains(&yaml_arg_str) {
                reports.push(KernelArgReport::new_created(
                    image_name.to_string(),
                    yaml_arg.clone(),
                ));
            }
        }

        // Find removed kernel args (in DB but not in YAML)
        for db_arg in db_kernel_args {
            let db_arg_str = db_arg.render_to_kernel_arg();
            if !yaml_args.contains(&db_arg_str) {
                reports.push(KernelArgReport::new_deleted(
                    image_name.to_string(),
                    db_arg.clone(),
                ));
            }
        }

        reports
    }
}

impl ImageYaml {
    pub(crate) async fn generate_image_report(
        &self,
        db_image: Option<Image>,
        db_kernel_args: Option<Vec<ImageKernelArg>>,
        flavor_map: &std::collections::HashMap<String, models::inventory::Flavor>,
    ) -> Result<ImageReport, InventoryError> {
        // If no DB image exists, this is a creation
        let Some(db_image) = db_image else {
            let kernel_arg_reports =
                KernelArg::generate_created_kernel_arg_reports(&self.name, &self.kernel_args);
            return Ok(ImageReport::new_created(self.clone(), kernel_arg_reports));
        };

        // error if names don't match
        if db_image.name != self.name {
            return Err(InventoryError::NotFound(format!(
                "Image name mismatch: expected '{}', got '{}'",
                self.name, db_image.name
            )));
        }

        let mut changes = ModifiedFields::new();

        // cobbler_name
        if db_image.cobbler_name != self.cobbler_name {
            changes.modified("cobbler_name", &db_image.cobbler_name, &self.cobbler_name)?;
        }

        // compare flavors (sorted vecs)
        let mut yaml_flavors = self.flavors.clone();
        yaml_flavors.sort();

        // Convert DB flavor IDs to names for comparison
        let mut db_flavor_names: Vec<String> = Vec::new();
        for flavor_id in &db_image.flavors {
            // Find the flavor by ID in the flavor_map
            if let Some(flavor) = flavor_map.values().find(|f| f.id == *flavor_id) {
                db_flavor_names.push(flavor.name.clone());
            }
        }
        db_flavor_names.sort();

        // Compare sorted flavor name lists
        if db_flavor_names != yaml_flavors {
            let db_display = if db_flavor_names.is_empty() {
                "(none)".to_string()
            } else {
                format!("\n        - {}", db_flavor_names.join("\n        - "))
            };
            let yaml_display = if yaml_flavors.is_empty() {
                "(none)".to_string()
            } else {
                format!("\n        - {}", yaml_flavors.join("\n        - "))
            };
            changes.modified(
                "flavors",
                &db_display,
                &yaml_display,
            )?;
        }

        // distro
        if db_image.distro != self.distro {
            changes.modified(
                "distro",
                db_image.distro.to_string(),
                self.distro.to_string(),
            )?;
        }

        // version
        if db_image.version != self.version {
            changes.modified("version", &db_image.version, &self.version)?;
        }

        // arch
        if db_image.arch != self.arch {
            changes.modified("arch", db_image.arch.to_string(), self.arch.to_string())?;
        }

        // http_unattended_install_config_path
        if db_image.http_unattended_install_config_path != self.http_unattended_install_config_path
        {
            changes.modified(
                "http_unattended_install_config_path",
                db_image.http_unattended_install_config_path.to_string(),
                self.http_unattended_install_config_path.to_string(),
            )?;
        }

        // http_iso_path
        if db_image.http_iso_path != self.http_iso_path {
            changes.modified(
                "http_iso_path",
                db_image.http_iso_path.to_string(),
                self.http_iso_path.to_string(),
            )?;
        }

        // tftp_kernel_path
        if db_image.tftp_kernel_path != self.tftp_kernel_path {
            changes.modified(
                "tftp_kernel_path",
                db_image.tftp_kernel_path.to_string(),
                self.tftp_kernel_path.to_string(),
            )?;
        }

        // tftp_initrd_paths
        if db_image.tftp_initrd_paths != self.tftp_initrd_paths {
            changes.modified(
                "tftp_initrd_paths",
                format!("{} paths", db_image.tftp_initrd_paths.len()),
                format!("{} paths", self.tftp_initrd_paths.len()),
            )?;
        }

        // kernel arg reports
        let kernel_arg_reports = KernelArg::generate_kernel_arg_reports(
            &self.name,
            &self.kernel_args,
            &db_kernel_args.unwrap_or_default(),
        );

        let kernel_args_changed = kernel_arg_reports.iter().any(|r| !r.is_unchanged());

        if changes.is_empty() && !kernel_args_changed {
            Ok(ImageReport::new_unchanged(self.name.clone()))
        } else {
            Ok(ImageReport::new_modified(
                self.clone(),
                changes,
                kernel_arg_reports,
            ))
        }
    }
}
