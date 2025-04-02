use dal::{DBTable, EasyTransaction, FKey};

use models::inventory::Vlan;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Hash, Serialize, Deserialize)]
pub struct VlanConnection {
    pub vlan: FKey<Vlan>,
    pub tagged: bool,
}

impl VlanConnection {
    pub async fn from_pair(t: &mut EasyTransaction<'_>, vlan_id: i16, tagged: bool) -> Self {
        Self {
            vlan: Self::fetch_vlan_id(vlan_id, t).await,
            tagged,
        }
    }
    pub async fn fetch_vlan_id(vlan_id: i16, t: &mut EasyTransaction<'_>) -> FKey<Vlan> {
        Vlan::select()
            .where_field("vlan_id")
            .equals(vlan_id)
            .run(t)
            .await
            .expect("Missing VLAN in database")
            .first()
            .unwrap()
            .id
    }
}
