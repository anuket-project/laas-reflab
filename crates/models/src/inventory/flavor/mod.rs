use dal::*;
use serde::{Deserialize, Serialize};
use std::str::FromStr;
use std::{collections::HashMap, fs::File, io::Write, path::PathBuf};

use crate::inventory::{Arch, DataValue};

mod extra_info;
mod interface;

pub use extra_info::ExtraFlavorInfo;
pub use interface::{CardType, InterfaceFlavor};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Flavor {
    pub id: FKey<Flavor>, // Flavor io used to create an instance

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

impl DBTable for Flavor {
    fn table_name() -> &'static str {
        "flavors"
    }

    fn id(&self) -> ID {
        self.id.into_id()
    }

    fn to_rowlike(&self) -> Result<HashMap<&str, Box<dyn ToSqlObject>>, anyhow::Error> {
        let c: [(&str, Box<dyn ToSqlObject>); _] = [
            ("id", Box::new(self.id)),
            ("arch", Box::new(self.arch.to_string())),
            ("name", Box::new(self.name.clone())),
            ("public", Box::new(self.public)),
            (
                "cpu_count",
                Box::new(serde_json::to_value(self.cpu_count as i64)?),
            ),
            ("ram", self.ram.to_sqlval()?),
            ("root_size", self.root_size.to_sqlval()?),
            ("disk_size", self.disk_size.to_sqlval()?),
            ("swap_size", self.swap_size.to_sqlval()?),
            ("brand", Box::new(self.brand.clone())),
            ("model", Box::new(self.model.clone())),
        ];
        Ok(c.into_iter().collect())
    }

    fn from_row(row: tokio_postgres::Row) -> Result<ExistingRow<Self>, anyhow::Error> {
        Ok(ExistingRow::from_existing(Self {
            id: row.try_get("id")?,
            arch: Arch::from_str(row.try_get("arch")?)?,
            name: row.try_get("name")?,
            public: row.try_get("public")?,
            cpu_count: serde_json::from_value::<i64>(row.try_get("cpu_count")?)?.min(0) as usize,
            ram: DataValue::from_sqlval(row.try_get("ram")?)?,
            root_size: DataValue::from_sqlval(row.try_get("root_size")?)?,
            disk_size: DataValue::from_sqlval(row.try_get("disk_size")?)?,
            swap_size: DataValue::from_sqlval(row.try_get("swap_size")?)?,
            brand: row.try_get("brand")?,
            model: row.try_get("model")?,
        }))
    }
}

impl Named for Flavor {
    fn name_columnnames() -> Vec<String> {
        vec!["name".to_owned()]
    }

    fn name_parts(&self) -> Vec<String> {
        vec![self.name.clone()]
    }
}

impl Lookup for Flavor {}

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

impl Flavor {
    pub async fn ports(
        &self,
        transaction: &mut EasyTransaction<'_>,
    ) -> Result<Vec<ExistingRow<InterfaceFlavor>>, anyhow::Error> {
        InterfaceFlavor::all_for_flavor(transaction, self.id).await
    }
}
