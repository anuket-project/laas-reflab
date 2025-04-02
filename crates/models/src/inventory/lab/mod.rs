use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use dal::{web::*, *};

#[derive(Serialize, Deserialize, Debug, Clone, Eq, PartialEq)]
pub struct Lab {
    pub id: FKey<Lab>,
    pub name: String,
    pub location: String,
    pub email: String,
    pub phone: String,
    pub is_dynamic: bool,
}

impl Lab {
    // pub async fn status(&self) -> Option<FKey<LabStatus>> {
    //     let mut client = new_client().await.expect("Expected to connect to db");
    //     let mut transaction = client
    //         .easy_transaction()
    //         .await
    //         .expect("Transaction creation error");
    //     let stati = LabStatus::select()
    //         .where_field("for_lab")
    //         .equals(self.id)
    //         .run(&mut transaction)
    //         .await
    //         .expect("Statuses for lab not found");
    //     match stati.len() {
    //         0 | 1 => stati.first().map(|s| s.id),
    //         _ => {
    //             let mut largest: ExistingRow<LabStatus> = stati
    //                 .first()
    //                 .expect("Expected to have a lab status")
    //                 .clone();
    //             for status in stati {
    //                 if largest.time.cmp(&status.time) == Ordering::Less {
    //                     largest = status;
    //                 }
    //             }
    //             Some(largest.id)
    //         }
    //     }
    // }
    //
    pub async fn get_by_name(
        transaction: &mut EasyTransaction<'_>,
        name: String,
    ) -> Result<Option<ExistingRow<Lab>>, anyhow::Error> {
        let tn = <Self as DBTable>::table_name();
        let q = format!("SELECT * FROM {tn} WHERE name = '{name}';");
        println!("{q}");

        let opt_row = transaction.query_opt(&q, &[]).await.anyway()?;
        Ok(match opt_row {
            None => None,
            Some(row) => Some(Self::from_row(row)?),
        })
    }
}

impl DBTable for Lab {
    fn table_name() -> &'static str {
        "labs"
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
            name: row.try_get("name")?,
            location: row.try_get("location")?,
            email: row.try_get("email")?,
            phone: row.try_get("phone")?,
            is_dynamic: row.try_get("is_dynamic")?,
        }))
    }

    fn to_rowlike(&self) -> Result<HashMap<&str, Box<dyn ToSqlObject>>, anyhow::Error> {
        let c: [(&str, Box<dyn ToSqlObject>); _] = [
            ("id", Box::new(self.id)),
            ("name", Box::new(self.name.clone())),
            ("location", Box::new(self.location.clone())),
            ("email", Box::new(self.email.clone())),
            ("phone", Box::new(self.phone.clone())),
            ("is_dynamic", Box::new(self.is_dynamic)),
        ];

        Ok(c.into_iter().collect())
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use proptest::prelude::*;
    use testing_utils::block_on_runtime;

    pub fn lab_strategy() -> impl Strategy<Value = Lab> {
        (
            any::<FKey<Lab>>(),                              // id
            "[a-zA-Z]{1, 20}",                               // name
            "[a-zA-Z]{1, 20}",                               // location
            r"[a-zA-Z0-9._%+-]+@[a-zA-Z0-9]+\.[a-zA-Z]{2,}", // email
            r"\d{10}",                                       // phone
            any::<bool>(),                                   // is_dynamic
        )
            .prop_map(|(id, name, location, email, phone, is_dynamic)| Lab {
                id,
                name: name.to_string(),
                location: location.to_string(),
                email: email.to_string(),
                phone: phone.to_string(),
                is_dynamic,
            })
    }

    proptest! {
        #[test]
        fn test_lab_model(lab in lab_strategy()) {
            block_on_runtime!({
                 let client = new_client().await;
                prop_assert!(client.is_ok(), "DB connection failed: {:?}", client.err());
                let mut client = client.unwrap();

                let transaction = client.easy_transaction().await;
                prop_assert!(transaction.is_ok(), "Transaction creation failed: {:?}", transaction.err());
                let mut transaction = transaction.unwrap();


                let new_row = NewRow::new(lab.clone());
                let insert_result = new_row.insert(&mut transaction).await;
                prop_assert!(insert_result.is_ok(), "Insert failed: {:?}", insert_result.err());

                let retrieved_lab_result = Lab::get_by_name(&mut transaction, lab.name.clone())
                .await;
                prop_assert!(retrieved_lab_result.is_ok(), "Retrieval failed: {:?}", retrieved_lab_result.err());
                let retrieved_lab = retrieved_lab_result.unwrap();

                prop_assert!(retrieved_lab.is_some());
                let retrieved = retrieved_lab.unwrap().into_inner();
                prop_assert_eq!(&retrieved, &lab);

                Ok(())
            })?
        }
    }
}
