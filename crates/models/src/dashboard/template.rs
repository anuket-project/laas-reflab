use common::prelude::reqwest::StatusCode;
use dal::{web::*, *};
use std::{fs::File, io::Write, path::PathBuf};

use common::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::{
    dashboard::{import_net, HostConfig, ImportHostConfig, Network, NetworkBlob},
    inventory::Lab,
};

#[derive(Serialize, Deserialize, Debug, Clone, JsonSchema, Eq, PartialEq, Default)]
pub struct Template {
    pub id: FKey<Template>,
    pub name: String,
    pub deleted: bool,
    pub description: String,
    pub owner: Option<String>,
    pub public: bool,                 // If template should be available to all users
    pub networks: Vec<FKey<Network>>, // User defined network
    pub hosts: Vec<HostConfig>,
    pub lab: FKey<Lab>,
}

#[derive(Serialize, Deserialize, Debug, Clone, JsonSchema)]
pub struct ImportTemplate {
    pub name: String,
    pub deleted: bool,
    pub description: String,
    pub owner: Option<String>,
    pub public: bool,               // If template should be available to all users
    pub networks: Vec<NetworkBlob>, // User defined network
    pub hosts: Vec<ImportHostConfig>,
    pub lab: String,
}

impl Template {
    async fn get_public_template(
        name: String,
        transaction: &mut EasyTransaction<'_>,
    ) -> Result<ExistingRow<Template>, anyhow::Error> {
        let res = Template::select()
            .where_field("name")
            .equals(name.clone())
            .where_field("public")
            .equals(true)
            .run(transaction)
            .await
            .expect("Expected to query for template");
        match res.len() {
            0 => Err(anyhow::Error::msg(format!(
                "Unable to find template with name: {name}"
            ))),
            1 => Ok(res.first().expect("Expected to find template").clone()),
            _ => Err(anyhow::Error::msg(format!(
                "Found multiple public templates with name: {name}"
            ))),
        }
    }

    pub async fn import(
        transaction: &mut EasyTransaction<'_>,
        import_file_path: std::path::PathBuf,
        proj_path: Option<PathBuf>,
    ) -> Result<Option<ExistingRow<Self>>, anyhow::Error> {
        match Lab::get_by_name(
            transaction,
            proj_path
                .clone()
                .expect("Expected project path")
                .file_name()
                .expect("Expected to find file name")
                .to_str()
                .expect("Expected host data dir for project to have a valid name")
                .to_owned(),
        )
        .await
        {
            Ok(opt_l) => {
                match opt_l {
                    Some(l) => l.id,
                    None => {
                        // In future import labs and try again
                        return Err(anyhow::Error::msg("Specified lab does not exist"));
                    }
                }
            }
            Err(_) => return Err(anyhow::Error::msg("Failed to find specified lab")),
        };

        let importtemplate: ImportTemplate =
            serde_json::from_reader(File::open(import_file_path)?)?;

        match importtemplate.public {
            true => {
                let mut template: Template = importtemplate
                    .to_template(transaction, proj_path.expect("Expected project path"))
                    .await;
                if let Ok(mut orig_template) =
                    Template::get_public_template(template.name.clone(), transaction).await
                {
                    template.id = orig_template.id;

                    orig_template.mass_update(template).unwrap();

                    orig_template
                        .update(transaction)
                        .await
                        .expect("Expected to update row");
                    Ok(Some(orig_template))
                } else {
                    let res = NewRow::new(template.clone())
                        .insert(transaction)
                        .await
                        .expect("Expected to create new row");
                    match res.get(transaction).await {
                        Ok(t) => Ok(Some(t)),
                        Err(e) => Err(anyhow::Error::msg(format!(
                            "Failed to insert template due to error: {}",
                            e
                        ))),
                    }
                }
            }
            false => Ok(None),
        }
    }

    pub async fn export(&self, transaction: &mut EasyTransaction<'_>) -> Result<(), anyhow::Error> {
        match self.public {
            true => {
                let lab_name = self
                    .lab
                    .get(transaction)
                    .await
                    .expect("Expected to find lab")
                    .name
                    .clone();

                let mut template_file_path = PathBuf::from(format!(
                    "./config_data/laas-hosts/inventory/labs/{}/templates/{}",
                    lab_name, self.name
                ));
                template_file_path.set_extension("json");

                let mut template_file =
                    File::create(template_file_path).expect("Expected to create template file");

                let import_template = ImportTemplate::from_template(transaction, self).await;

                match template_file
                    .write_all(serde_json::to_string_pretty(&import_template)?.as_bytes())
                {
                    Ok(_) => Ok(()),
                    Err(_) => Err(anyhow::Error::msg(format!(
                        "Failed to export host {}",
                        self.name.clone()
                    ))),
                }
            }
            false => Ok(()), // Do not export non-public templates
        }
    }
}

impl DBTable for Template {
    fn table_name() -> &'static str {
        "templates"
    }

    fn id(&self) -> ID {
        self.id.into_id()
    }

    fn id_mut(&mut self) -> &mut ID {
        self.id.into_id_mut()
    }

    fn from_row(row: tokio_postgres::Row) -> Result<ExistingRow<Self>, anyhow::Error> {
        Ok(ExistingRow::from_existing(Self {
            id: row.try_get("id")?,
            owner: row.try_get("owner")?,
            name: row.try_get("name")?,
            deleted: row.try_get("deleted")?,
            public: row.try_get("public")?,
            description: row.try_get("description")?,
            networks: row.try_get("networks")?,
            hosts: serde_json::from_value(row.try_get("hosts")?)?,
            lab: row.try_get("lab")?,
        }))
    }

    fn to_rowlike(&self) -> Result<HashMap<&str, Box<dyn ToSqlObject>>, anyhow::Error> {
        let clone = self.clone();
        let c: [(&str, Box<dyn ToSqlObject>); _] = [
            ("id", Box::new(clone.id)),
            ("owner", Box::new(clone.owner)),
            ("name", Box::new(clone.name)),
            ("deleted", Box::new(clone.deleted)),
            ("public", Box::new(clone.public)),
            ("description", Box::new(clone.description)),
            ("networks", Box::new(clone.networks)),
            ("hosts", Box::new(serde_json::to_value(clone.hosts)?)),
            ("lab", Box::new(clone.lab)),
        ];

        Ok(c.into_iter().collect())
    }
}

impl Template {
    pub async fn get_public(t: &mut EasyTransaction<'_>) -> Result<Vec<Template>, anyhow::Error> {
        let table_name = <Template as DBTable>::table_name();

        let query = format!("SELECT * FROM {table_name} WHERE public = $1");
        let qr = t.query(&query, &[&true]).await?;

        let results: Vec<Template> = qr
            .into_iter()
            .filter_map(|row| {
                Template::from_row(row)
                    .log_error(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "database corruption did not allow instantiating a template",
                        true,
                    )
                    .map(|er| er.into_inner())
                    .ok()
            })
            .collect();

        Ok(results)
    }

    pub async fn get_all(t: &mut EasyTransaction<'_>) -> Result<Vec<Template>, anyhow::Error> {
        let table_name = Template::table_name();

        let query = format!("SELECT * FROM {table_name} WHERE deleted = $1;");
        let qr = t.query(&query, &[&false]).await?;

        let results: Vec<Template> = qr
            .into_iter()
            .filter_map(|row| {
                Template::from_row(row)
                    .log_error(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "database corruption did not allow instantiating a template",
                        true,
                    )
                    .map(|er| er.into_inner())
                    .ok()
            })
            .collect();

        Ok(results)
    }

    pub async fn get_by_lab(
        t: &mut EasyTransaction<'_>,
        name: String,
    ) -> Result<Vec<ExistingRow<Template>>, anyhow::Error> {
        let table_name = Template::table_name();

        let query = format!("SELECT * FROM {table_name} WHERE name = $1;");
        let rows = t.query(&query, &[&name]).await?;
        let vals: Result<Vec<_>, anyhow::Error> =
            rows.into_iter().map(Template::from_row).collect();

        let vals = vals?;

        Ok(vals)
    }

    pub async fn owned_by(
        t: &mut EasyTransaction<'_>,
        owner: String,
    ) -> Result<Vec<Template>, anyhow::Error> {
        let table_name = Template::table_name();
        let query = format!("SELECT * FROM {table_name} WHERE owner = $1;");

        let qr = t.query(&query, &[&owner]).await.anyway()?;

        let results: Vec<Template> = qr
            .into_iter()
            .filter_map(|row| {
                Template::from_row(row)
                    .log_error(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "database corruption did not allow instantiating a template",
                        true,
                    )
                    .map(|er| er.into_inner())
                    .ok()
            })
            .collect();

        Ok(results)
    }
}

impl ImportTemplate {
    pub async fn to_template(
        &self,
        transaction: &mut EasyTransaction<'_>,
        _proj_path: PathBuf,
    ) -> Template {
        let clone = self.clone();

        let lab = Lab::get_by_name(transaction, clone.lab)
            .await
            .expect("Expected to find lab")
            .expect("Expected that lab exists");

        let mut nets: Vec<FKey<Network>> = Vec::new();
        for net in clone.networks {
            let id = import_net(net, transaction).await;
            nets.push(id);
        }

        let mut hosts: Vec<HostConfig> = Vec::new();
        for host_config in clone.hosts.clone() {
            hosts.push(host_config.to_host_config(transaction).await);
        }

        Template {
            id: FKey::new_id_dangling(),
            name: clone.name,
            public: clone.public,
            deleted: clone.deleted,
            description: clone.description,
            owner: clone.owner,
            networks: nets,
            hosts,
            lab: lab.id,
        }
    }

    pub async fn from_template(
        transaction: &mut EasyTransaction<'_>,
        template: &Template,
    ) -> ImportTemplate {
        let clone = template.clone();
        let lab = clone
            .lab
            .get(transaction)
            .await
            .expect("Expected to find lab");
        let mut networks: Vec<NetworkBlob> = Vec::new();

        for net_key in clone.networks {
            let net = net_key
                .get(transaction)
                .await
                .expect("Expected to find network");
            networks.push(NetworkBlob {
                name: net.name.clone(),
                public: net.public,
            })
        }

        let mut hosts: Vec<ImportHostConfig> = Vec::new();
        for host_config in clone.hosts.clone() {
            hosts.push(ImportHostConfig::from_host_config(transaction, &host_config).await);
        }

        ImportTemplate {
            name: clone.name,
            deleted: clone.deleted,
            description: clone.description,
            owner: clone.owner,
            public: clone.public,
            networks,
            hosts,
            lab: lab.name.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::collection::vec;
    use proptest::prelude::*;
    use testing_utils::block_on_runtime;

    impl Arbitrary for Template {
        type Strategy = BoxedStrategy<Self>;
        type Parameters = ();

        fn arbitrary_with(_args: Self::Parameters) -> Self::Strategy {
            (
                any::<FKey<Template>>(),            // id
                any::<String>(),                    // name
                any::<bool>(),                      // deleted
                any::<String>(),                    // description
                any::<Option<String>>(),            // owner
                any::<bool>(),                      // public
                vec(any::<FKey<Network>>(), 0..10), // networks
                vec(any::<HostConfig>(), 0..10),    // hosts
                any::<FKey<Lab>>(),                 // lab
            )
                .prop_map(
                    |(id, name, deleted, description, owner, public, networks, hosts, lab)| {
                        Template {
                            id,
                            name,
                            deleted,
                            description,
                            owner,
                            public,
                            networks,
                            hosts,
                            lab,
                        }
                    },
                )
                .boxed()
        }
    }

    proptest! {
        #[test]
        fn first_template_model(template in any::<Template>()) {
            block_on_runtime!({
                let client = new_client().await;
                prop_assert!(client.is_ok(), "DB connection failed: {:?}", client.err());
                let mut client = client.unwrap();

                let transaction = client.easy_transaction().await;
                prop_assert!(transaction.is_ok(), "Transaction creation failed: {:?}", transaction.err());
                let mut transaction = transaction.unwrap();


                let new_row = NewRow::new(template.clone());
                let insert_result = new_row.insert(&mut transaction).await;
                prop_assert!(insert_result.is_ok(), "Insert failed: {:?}", insert_result.err());

                let retrieved_template_result = Template::select().where_field("id").equals(template.id).run(&mut transaction)
                    .await;
                prop_assert!(retrieved_template_result.is_ok(), "Retrieval failed: {:?}", retrieved_template_result.err());

                let first_template = retrieved_template_result.unwrap().into_iter().next();
                prop_assert!(first_template.is_some(), "No host found, empty result");

                let retrieved_template = first_template.unwrap().clone().into_inner();
                prop_assert_eq!(retrieved_template, template);

                Ok(())
            })?
        }
    }
}
