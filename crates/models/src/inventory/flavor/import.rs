use crate::inventory::{Arch, DataValue, Flavor};
use dal::{EasyTransaction, ExistingRow, FKey, Importable, Lookup, Named, NewRow};
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::Write;
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ImportFlavor {
    pub arch: Arch,
    pub name: String,
    pub public: bool,
    pub cpu_count: usize,
    pub ram: DataValue,
    pub root_size: DataValue,
    pub disk_size: DataValue,
    pub swap_size: DataValue,
    pub brand: String,
    pub model: String,
}

impl ImportFlavor {
    pub async fn to_flavor(&self, _transaction: &mut EasyTransaction<'_>) -> Flavor {
        let clone = self.clone();

        Flavor {
            id: FKey::new_id_dangling(),
            arch: clone.arch,
            name: clone.name,
            public: clone.public,
            cpu_count: clone.cpu_count,
            ram: clone.ram,
            root_size: clone.root_size,
            disk_size: clone.disk_size,
            swap_size: clone.swap_size,
            brand: clone.brand,
            model: clone.model,
        }
    }

    pub fn from_flavor(flavor: &Flavor) -> ImportFlavor {
        ImportFlavor {
            arch: flavor.arch,
            name: flavor.name.clone(),
            public: flavor.public,
            cpu_count: flavor.cpu_count,
            ram: flavor.ram,
            root_size: flavor.root_size,
            disk_size: flavor.disk_size,
            swap_size: flavor.swap_size,
            brand: flavor.brand.clone(),
            model: flavor.model.clone(),
        }
    }
}

impl Importable for Flavor {
    async fn import(
        transaction: &mut EasyTransaction<'_>,
        import_file_path: std::path::PathBuf,
        _proj_path: Option<PathBuf>,
    ) -> Result<Option<ExistingRow<Self>>, anyhow::Error> {
        let importflavor: ImportFlavor = serde_json::from_reader(File::open(import_file_path)?)?;
        let mut flavor: Flavor = importflavor.to_flavor(transaction).await;

        if let Ok(mut orig_flavor) = Flavor::lookup(transaction, Flavor::name_parts(&flavor)).await
        {
            flavor.id = orig_flavor.id;

            orig_flavor.mass_update(flavor).unwrap();

            orig_flavor
                .update(transaction)
                .await
                .expect("Expected to update row");
            Ok(Some(orig_flavor))
        } else {
            let res = NewRow::new(flavor.clone())
                .insert(transaction)
                .await
                .expect("Expected to create new row");

            match res.get(transaction).await {
                Ok(f) => Ok(Some(f)),
                Err(e) => Err(anyhow::Error::msg(format!(
                    "Failed to insert flavor due to error: {}",
                    e
                ))),
            }
        }
    }

    async fn export(&self, _transaction: &mut EasyTransaction<'_>) -> Result<(), anyhow::Error> {
        let flavor_dir = PathBuf::from("./config_data/laas-hosts/inventory/flavors");
        let mut flavor_file_path = flavor_dir;
        flavor_file_path.push(self.name.clone());
        flavor_file_path.set_extension("json");
        let mut flavor_file =
            File::create(flavor_file_path).expect("Expected to create flavor file");

        let import_flavor = ImportFlavor::from_flavor(self);

        match flavor_file.write_all(serde_json::to_string_pretty(&import_flavor)?.as_bytes()) {
            Ok(_) => Ok(()),
            Err(_) => Err(anyhow::Error::msg(format!(
                "Failed to export flavor {}",
                self.name.clone()
            ))),
        }
    }
}
