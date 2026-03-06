use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

#[derive(Serialize, Deserialize, Debug, Clone, Eq, PartialEq, sqlx::FromRow)]
/// FlavorCommands are used to define special overrides for provision time rendered autoinstall files 
/// based on a Flavor / Image combination.
/// For example, consider an override for Ampere HR350A servers on Ubuntu 22.04 to add a run command to uninstall fwupd.
pub struct FlavorCommands {
    pub for_flavor: Uuid,
    pub for_image: Uuid,
    pub commands: Vec<String>,
}

impl FlavorCommands {
    pub async fn get_for_flavor_id(
        for_flavor_id: &Uuid,
        for_image_id: &Uuid,
        pool: &PgPool,
    ) -> Result<FlavorCommands, sqlx::Error> {
        sqlx::query_as!(
            FlavorCommands,
            "SELECT * FROM flavor_commands WHERE for_flavor = $1 and for_image = $2 LIMIT 1",
            for_flavor_id,
            for_image_id
        )
        .fetch_one(pool)
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_todo() {
        todo!()
    }
}
