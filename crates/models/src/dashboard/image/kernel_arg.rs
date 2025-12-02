use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

#[derive(Serialize, Deserialize, Debug, Clone, Eq, PartialEq, sqlx::FromRow)]
pub struct ImageKernelArg {
    pub id: Uuid,
    pub for_image: Uuid,
    pub _key: String,
    pub _value: Option<String>,
}

impl ImageKernelArg {
    pub fn render_to_kernel_arg(&self) -> String {
        match &self._value {
            Some(v) => format!("{}={}", self._key, v),
            None => self._key.clone(),
        }
    }

    pub async fn compile_kernel_args_for_image(
        image_name: &str,
        pool: &PgPool,
    ) -> Result<Vec<String>, sqlx::Error> {
        let kernel_args: Vec<ImageKernelArg> = sqlx::query_as!(
            ImageKernelArg,
            r#"
            SELECT *
            FROM image_kernel_args
            WHERE for_image = (SELECT id FROM images WHERE name = $1)
            ORDER BY _key ASC;
            "#,
            image_name
        )
        .fetch_all(pool)
        .await?;

        Ok(kernel_args
            .into_iter()
            .map(|arg| arg.render_to_kernel_arg())
            .collect())
    }
}
