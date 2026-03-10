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
    /// Fetches the related FlavorCommands for a given Flavor / Image combination.
    /// Returns None if no such entry exists.
    pub async fn get_for_flavor_image_ids(
        for_flavor_id: &Uuid,
        for_image_id: &Uuid,
        pool: &PgPool,
    ) -> Result<Option<FlavorCommands>, sqlx::Error> {

        sqlx::query_as!(
            FlavorCommands,
            "SELECT * FROM flavor_commands WHERE for_flavor = $1 and for_image = $2 LIMIT 1",
            for_flavor_id,
            for_image_id
        )
        .fetch_optional(pool)
        .await
    }

    /// Inserts the FlavorCommands and returns the newly inserted row as a struct.
    pub async fn set_for_flavor_image_ids(
        for_flavor_id: &Uuid,
        for_image_id: &Uuid,
        commands: Vec<String>,
        pool: &PgPool,
    ) -> Result<FlavorCommands, sqlx::Error> {

        sqlx::query_as!(
            FlavorCommands,
            r#"
            INSERT INTO flavor_commands (for_flavor, for_image, commands)
            VALUES ($1, $2, $3)
            RETURNING for_flavor, for_image, commands
            "#,
            for_flavor_id,
            for_image_id,
            &commands
        )
        .fetch_one(pool)
        .await
    }

    /// Deletes the FlavorCommands for a Flavor / Image combination.
    pub async fn delete_for_flavor_image_ids(
        for_flavor_id: &Uuid,
        for_image_id: &Uuid,
        pool: &PgPool,
    ) -> Result<(), sqlx::Error> {

        sqlx::query!(
            r#"
            DELETE FROM flavor_commands WHERE for_flavor = $1 and for_image = $2
            "#,
            for_flavor_id,
            for_image_id,
        )
        .execute(pool)
        .await?;

        Ok(())
    }
}

impl std::fmt::Display for FlavorCommands {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {

        write!(f, "{}", self.commands.join("\n"))
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
