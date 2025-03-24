use dal::{web::AnyWay, *};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

mod os;
mod port;

pub use os::SwitchOS;
pub use port::SwitchPort;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Switch {
    pub id: FKey<Switch>,

    pub name: String,
    pub ip: String,
    pub user: String,
    pub pass: String,
    pub switch_os: Option<FKey<SwitchOS>>,
    pub management_vlans: Vec<i16>,
    pub ipmi_vlan: i16,
    pub public_vlans: Vec<i16>,
}

impl PartialEq for Switch {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
            && self.name == other.name
            && self.ip == other.ip
            && self.user == other.user
            && self.pass == other.pass
            && self.switch_os == other.switch_os
    }
}

impl DBTable for Switch {
    fn id(&self) -> ID {
        self.id.into_id()
    }

    fn id_mut(&mut self) -> &mut ID {
        self.id.into_id_mut()
    }

    fn table_name() -> &'static str {
        "switches"
    }
    // JSONMODEL -> DBTABLE
    fn from_row(row: tokio_postgres::Row) -> Result<ExistingRow<Self>, anyhow::Error> {
        Ok(ExistingRow::from_existing(Self {
            id: row.try_get("id")?,
            name: row.try_get("name")?,
            ip: row.try_get("ip")?,
            user: row.try_get("switch_user")?,
            pass: row.try_get("switch_pass")?,
            switch_os: row.try_get("switch_os")?,
            management_vlans: row.try_get("management_vlans")?,
            ipmi_vlan: row.try_get("ipmi_vlan")?,
            public_vlans: row.try_get("public_vlans")?,
        }))
    }

    fn to_rowlike(&self) -> Result<HashMap<&str, Box<dyn ToSqlObject>>, anyhow::Error> {
        let clone = self.clone();
        let c: [(&str, Box<dyn ToSqlObject>); _] = [
            ("id", Box::new(clone.id)),
            ("name", Box::new(clone.name)),
            ("ip", Box::new(clone.ip)),
            ("switch_user", Box::new(clone.user)),
            ("switch_pass", Box::new(clone.pass)),
            ("switch_os", Box::new(clone.switch_os)),
            ("management_vlans", Box::new(clone.management_vlans)),
            ("ipmi_vlan", Box::new(clone.ipmi_vlan)),
            ("public_vlans", Box::new(clone.public_vlans)),
        ];

        Ok(c.into_iter().collect())
    }
}

impl Switch {
    pub async fn get_by_ip(
        transaction: &mut EasyTransaction<'_>,
        ip: String,
    ) -> Result<Option<ExistingRow<Switch>>, anyhow::Error> {
        let tn = <Self as DBTable>::table_name();
        let q = format!("SELECT * FROM {tn} WHERE ip = $1;");
        let opt_row = transaction.query_opt(&q, &[&ip]).await.anyway()?;
        Ok(match opt_row {
            None => None,
            Some(row) => Some(Self::from_row(row)?),
        })
    }

    pub async fn get_by_name(
        transaction: &mut EasyTransaction<'_>,
        name: String,
    ) -> Result<Option<ExistingRow<Switch>>, anyhow::Error> {
        let tn = <Self as DBTable>::table_name();
        let q = format!("SELECT * FROM {tn} WHERE name = $1;");

        let opt_row = transaction.query_opt(&q, &[&name]).await.anyway()?;
        Ok(match opt_row {
            None => None,
            Some(row) => Some(Self::from_row(row)?),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::collection::vec;
    use proptest::option::of;
    use proptest::prelude::*;
    use testing_utils::block_on_runtime;

    impl Arbitrary for Switch {
        type Parameters = ();
        type Strategy = BoxedStrategy<Self>;

        fn arbitrary_with(_: Self::Parameters) -> Self::Strategy {
            (
                any::<FKey<Switch>>(),                         // id
                any::<String>(),                               // name
                any::<String>(),                               // ip
                any::<String>(),                               // user
                any::<String>(),                               // pass
                of(Just(FKey::<SwitchOS>::new_id_dangling())), // switch_os
                vec(any::<i16>(), 0..10),                      // management_vlans
                any::<i16>(),                                  // ipmi_vlan
                vec(any::<i16>(), 0..10),                      // public_vlans
            )
                .prop_map(
                    |(
                        id,
                        name,
                        ip,
                        user,
                        pass,
                        switch_os,
                        management_vlans,
                        ipmi_vlan,
                        public_vlans,
                    )| {
                        Switch {
                            id,
                            name,
                            ip,
                            user,
                            pass,
                            switch_os,
                            management_vlans,
                            ipmi_vlan,
                            public_vlans,
                        }
                    },
                )
                .boxed()
        }
    }

    proptest! {
        #[test]
        fn test_switch_model(switch in any::<Switch>()) {
            block_on_runtime!({
                let client = new_client().await;
                prop_assert!(client.is_ok(), "DB connection failed: {:?}", client.err());
                let mut client = client.unwrap();

                let transaction = client.easy_transaction().await;
                prop_assert!(transaction.is_ok(), "Transaction creation failed: {:?}", transaction.err());
                let mut transaction = transaction.unwrap();

                let new_row = NewRow::new(switch.clone());
                let insert_result = new_row.insert(&mut transaction).await;
                prop_assert!(insert_result.is_ok(), "Insert failed: {:?}", insert_result.err());

                let retrieved_switch = Switch::select()
                    .where_field("id")
                    .equals(switch.id)
                    .run(&mut transaction)
                    .await;

                prop_assert!(retrieved_switch.is_ok(), "Retrieval failed: {:?}", retrieved_switch.err());
                let retrieved_switch = retrieved_switch.unwrap();

                let first_switch = retrieved_switch.first();
                prop_assert!(first_switch.is_some(), "No switch found, empty result");

                let retrieved_switch = first_switch.unwrap().clone().into_inner();

                prop_assert_eq!(retrieved_switch, switch);

                Ok(())
            })?
        }
    }
}
