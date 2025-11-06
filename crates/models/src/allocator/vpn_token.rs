use dal::{web::*, *};

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use common::prelude::*;

// TODO: Delete this bc it should not exist
#[derive(Serialize, Deserialize, Debug, Clone, Hash, Eq, PartialEq)]
pub struct VPNToken {
    pub id: FKey<VPNToken>,
    pub username: String,
    pub project: String,
}

impl DBTable for VPNToken {
    fn table_name() -> &'static str {
        "vpn_tokens"
    }

    fn id(&self) -> ID {
        self.id.into_id()
    }

    fn id_mut(&mut self) -> &mut ID {
        self.id.into_id_mut()
    }

    fn from_row(row: tokio_postgres::Row) -> Result<ExistingRow<Self>, anyhow::Error> {
        let id = row.try_get("id").anyway()?;
        let username = row.try_get("username").anyway()?;
        let project = row.try_get("project").anyway()?;

        Ok(ExistingRow::from_existing(Self {
            id,
            username,
            project,
        }))
    }

    fn to_rowlike(&self) -> Result<HashMap<&str, Box<dyn ToSqlObject>>, anyhow::Error> {
        let Self {
            id,
            username,
            project,
        } = self.clone();
        let c: [(&str, Box<dyn ToSqlObject>); _] = [
            ("id", Box::new(id)),
            ("username", Box::new(username)),
            ("project", Box::new(project)),
        ];

        Ok(c.into_iter().collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use testing_utils::block_on_runtime;

    impl Arbitrary for VPNToken {
        type Parameters = ();
        type Strategy = BoxedStrategy<Self>;

        fn arbitrary_with(_args: Self::Parameters) -> Self::Strategy {
            (
                any::<FKey<VPNToken>>(), // id
                any::<String>(),         // username
                any::<String>(),         // project
            )
                .prop_map(|(id, username, project)| VPNToken {
                    id,
                    username,
                    project,
                })
                .boxed()
        }
    }

    proptest! {
        #[test]
        fn test_vpn_token_model(vpn_token in VPNToken::arbitrary()) {
            block_on_runtime!({
                let client = new_client().await;
                prop_assert!(client.is_ok(), "DB connection failed: {:?}", client.err());
                let mut client = client.unwrap();

                let transaction = client.easy_transaction().await;
                prop_assert!(transaction.is_ok(), "Transaction creation failed: {:?}", transaction.err());
                let mut transaction = transaction.unwrap();

                let new_row = NewRow::new(vpn_token.clone());
                let insert_result = new_row.insert(&mut transaction).await;
                prop_assert!(insert_result.is_ok(), "Insert failed: {:?}", insert_result.err());

                let retrieved_result = VPNToken::select()
                    .where_field("id")
                    .equals(vpn_token.id)
                    .run(&mut transaction)
                    .await;

                prop_assert!(retrieved_result.is_ok(), "Retrieval failed: {:?}", retrieved_result.err());
                let retrieved_tokens = retrieved_result.unwrap();

                let retrieved_token = retrieved_tokens.first();
                prop_assert!(retrieved_token.is_some(), "No Allocation found, empty result");

                let retrieved_token = retrieved_token.unwrap().clone().into_inner();
                prop_assert_eq!(retrieved_token, vpn_token);

                Ok(())

            })?
        }
    }
}
