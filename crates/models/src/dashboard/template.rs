use common::prelude::reqwest::StatusCode;
use dal::{web::*, *};

use common::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::{
    dashboard::{HostConfig, Network},
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
        fn test_template_model(template in any::<Template>()) {
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
