use common::prelude::reqwest::StatusCode;
use dal::{web::*, *};
use std::{fs::File, io::Write, path::PathBuf};

use common::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::inventory::Flavor;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Image {
    pub id: FKey<Image>, // id of image used for booking

    pub owner: String,
    pub name: String, // name of image
    pub deleted: bool,
    pub cobbler_name: String,
    pub public: bool,
    pub flavors: Vec<FKey<Flavor>>, // vector of compatible flavor IDs
}

impl Named for Image {
    fn name_columnnames() -> Vec<std::string::String> {
        vec!["name".to_owned()]
    }

    fn name_parts(&self) -> Vec<String> {
        vec![self.name.clone()]
    }
}

impl Lookup for Image {}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ImportImage {
    pub owner: String,
    pub name: String,
    pub deleted: bool,
    pub cobbler_name: String,
    pub public: bool,
    pub flavors: Vec<String>,
}

impl ImportImage {
    pub async fn to_image(&self, transaction: &mut EasyTransaction<'_>) -> Image {
        let mut flavors: Vec<FKey<Flavor>> = Vec::new();
        for flavor in self.flavors.clone() {
            let mut flavor_path = PathBuf::from("./config_data/laas-hosts/inventory/flavors");
            flavor_path.push(flavor.as_str());
            flavor_path.set_extension("json");
            flavors.push(
                Flavor::import(transaction, flavor_path.clone(), None)
                    .await
                    .expect("Expected to import flavor at {flavor_path:?}")
                    .unwrap()
                    .id,
            )
        }

        let clone = self.clone();

        Image {
            id: FKey::new_id_dangling(),
            owner: clone.owner,
            name: clone.name,
            deleted: clone.deleted,
            cobbler_name: clone.cobbler_name,
            public: clone.public,
            flavors,
        }
    }

    pub async fn from_image(transaction: &mut EasyTransaction<'_>, image: Image) -> ImportImage {
        let clone = image.clone();
        let mut flavors = Vec::new();
        for flavor in clone.flavors {
            tracing::info!(
                "getting flavor name for flavor: {:?}  for image: {}",
                flavor,
                clone.name
            );
            flavors.push(
                flavor
                    .get(transaction)
                    .await
                    .expect("Expected to get flavor from FKey")
                    .name
                    .clone(),
            );
            tracing::info!("pushed flavor name to vec");
        }

        ImportImage {
            owner: clone.owner,
            name: clone.name,
            deleted: clone.deleted,
            cobbler_name: clone.cobbler_name,
            public: clone.public,
            flavors,
        }
    }
}

impl Importable for Image {
    async fn import(
        transaction: &mut EasyTransaction<'_>,
        import_file_path: std::path::PathBuf,
        _proj_path: Option<PathBuf>,
    ) -> Result<Option<ExistingRow<Self>>, anyhow::Error> {
        let importimage: ImportImage = serde_json::from_reader(File::open(import_file_path)?)?;
        let mut image: Image = importimage.to_image(transaction).await;

        if let Ok(mut orig_image) = Image::lookup(transaction, Image::name_parts(&image)).await {
            image.id = orig_image.id;

            orig_image.mass_update(image).unwrap();

            orig_image
                .update(transaction)
                .await
                .expect("Expected to update row");
            Ok(Some(orig_image))
        } else {
            let res = NewRow::new(image.clone())
                .insert(transaction)
                .await
                .expect("Expected to create new row");

            match res.get(transaction).await {
                Ok(i) => Ok(Some(i)),
                Err(e) => Err(anyhow::Error::msg(format!(
                    "Failed to import image due to error: {}",
                    e
                ))),
            }
        }
    }

    async fn export(&self, transaction: &mut EasyTransaction<'_>) -> Result<(), anyhow::Error> {
        let image_dir = PathBuf::from("./config_data/laas-hosts/inventory/images");
        let mut image_file_path = image_dir;
        image_file_path.push(self.name.clone());
        image_file_path.set_extension("json");
        let mut image_file = File::create(image_file_path).expect("Expected to create image file");

        let import_image = ImportImage::from_image(transaction, self.clone()).await;

        match image_file.write_all(serde_json::to_string_pretty(&import_image)?.as_bytes()) {
            Ok(_) => Ok(()),
            Err(_) => Err(anyhow::Error::msg(format!(
                "Failed to export image {}",
                self.name.clone()
            ))),
        }
    }
}

impl DBTable for Image {
    fn id(&self) -> ID {
        self.id.into_id()
    }

    fn table_name() -> &'static str {
        "images"
    }

    fn from_row(row: tokio_postgres::Row) -> Result<ExistingRow<Self>, anyhow::Error> {
        Ok(ExistingRow::from_existing(Self {
            id: row.try_get("id")?,
            owner: row.try_get("owner")?,
            name: row.try_get("name")?,
            deleted: row.try_get("deleted")?,
            cobbler_name: row.try_get("cobbler_name")?,
            public: row.try_get("public")?,
            flavors: row.try_get("flavors")?,
        }))
    }

    fn to_rowlike(&self) -> Result<HashMap<&str, Box<dyn ToSqlObject>>, anyhow::Error> {
        let clone = self.clone();
        let c: [(&str, Box<dyn ToSqlObject>); _] = [
            ("id", Box::new(clone.id)),
            ("owner", Box::new(clone.owner)),
            ("name", Box::new(clone.name)),
            ("deleted", Box::new(clone.deleted)),
            ("cobbler_name", Box::new(clone.cobbler_name)),
            ("public", Box::new(clone.public)),
            ("flavors", Box::new(clone.flavors)),
        ];

        Ok(c.into_iter().collect())
    }
}

impl Image {
    pub async fn get_by_name(
        t: &mut EasyTransaction<'_>,
        name: String,
    ) -> Result<ExistingRow<Image>, anyhow::Error> {
        let table_name = Self::table_name();
        let query = format!("SELECT * FROM {table_name} WHERE name = $1;");
        let qr = t.query_opt(&query, &[&name]).await?;
        let qr = qr.ok_or(anyhow::Error::msg("Image did not exist for query"))?;

        let results = Image::from_row(qr)
            .log_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "database corruption did not allow instantiating an image from a row",
                true,
            )
            .map(|i| i.into_inner())
            .unwrap(); // TODO: get rid of unwrap

        Ok(ExistingRow::from_existing(results))
    }

    pub async fn images_for_flavor(
        t: &mut EasyTransaction<'_>,
        flavor: FKey<Flavor>,
        owner: Option<String>,
    ) -> Result<Vec<Image>, anyhow::Error> {
        if owner.is_some() {
            let table_name = Self::table_name();
            let query = format!("SELECT * FROM {table_name} WHERE (owner = $1 OR public = $2) AND ($3 = ANY(flavors));");
            let qr = t.query(&query, &[&owner, &true, &flavor.into_id()]).await?;

            let results: Vec<Image> = qr
                .into_iter()
                .filter_map(|row| {
                    Image::from_row(row)
                        .log_error(
                            StatusCode::INTERNAL_SERVER_ERROR,
                            "database corruption did not allow instantiating an image from a row",
                            true,
                        )
                        .map(|er| er.into_inner())
                        .ok()
                })
                .collect();

            Ok(results)
        } else {
            let table_name = Self::table_name();
            let query =
                format!("SELECT * FROM {table_name} WHERE (public = $1) AND ($2 = ANY(flavors));");
            let qr = t.query(&query, &[&true, &flavor.into_id()]).await?;

            let results: Vec<Image> = qr
                .into_iter()
                .filter_map(|row| {
                    Image::from_row(row)
                        .log_error(
                            StatusCode::INTERNAL_SERVER_ERROR,
                            "database corruption did not allow instantiating an image from a row",
                            true,
                        )
                        .map(|er| er.into_inner())
                        .ok()
                })
                .collect();

            Ok(results)
        }
    }
}
