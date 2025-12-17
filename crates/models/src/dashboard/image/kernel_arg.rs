use config::settings;
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
    /// Renders the kernel arg as it appears in the database (no replacements)
    pub fn render_to_kernel_arg(&self) -> String {
        match &self._value {
            Some(v) => format!("{}={}", self._key, v),
            None => self._key.clone(),
        }
    }

    /// Renders the kernel arg with {{PXE_SERVER}} placeholder replaced with actual PXE server address
    pub fn render_to_kernel_arg_with_pxe_replacement(&self) -> String {
        let pxe_server = &settings().pxe.address;
        self.render_to_kernel_arg_with_replacement(pxe_server)
    }

    /// Helper method that replaces {{PXE_SERVER}} with a provided server address
    /// This is useful for testing without requiring config to be loaded
    fn render_to_kernel_arg_with_replacement(&self, pxe_server: &str) -> String {
        match &self._value {
            Some(v) => {
                let replaced_value = v.replace("{{PXE_SERVER}}", pxe_server);
                format!("{}={}", self._key, replaced_value)
            }
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
            .map(|arg| arg.render_to_kernel_arg_with_pxe_replacement())
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_render_to_kernel_arg_without_replacement() {
        let arg = ImageKernelArg {
            id: Uuid::new_v4(),
            for_image: Uuid::new_v4(),
            _key: "initrd".to_string(),
            _value: Some("tftp://{{PXE_SERVER}}/images/initrd.img".to_string()),
        };

        let rendered = arg.render_to_kernel_arg();
        assert_eq!(rendered, "initrd=tftp://{{PXE_SERVER}}/images/initrd.img");
    }

    #[test]
    fn test_render_to_kernel_arg_with_pxe_replacement() {
        let arg = ImageKernelArg {
            id: Uuid::new_v4(),
            for_image: Uuid::new_v4(),
            _key: "initrd".to_string(),
            _value: Some("tftp://{{PXE_SERVER}}/images/initrd.img".to_string()),
        };

        let pxe_server = "192.168.1.100";
        let rendered = arg.render_to_kernel_arg_with_replacement(pxe_server);

        assert_eq!(rendered, "initrd=tftp://192.168.1.100/images/initrd.img");
        assert!(!rendered.contains("{{PXE_SERVER}}"));
    }

    #[test]
    fn test_render_to_kernel_arg_with_multiple_placeholders() {
        let arg = ImageKernelArg {
            id: Uuid::new_v4(),
            for_image: Uuid::new_v4(),
            _key: "ks".to_string(),
            _value: Some("http://{{PXE_SERVER}}/kickstarts/{{PXE_SERVER}}/server.ks".to_string()),
        };

        let pxe_server = "192.168.1.100";
        let rendered = arg.render_to_kernel_arg_with_replacement(pxe_server);

        assert_eq!(rendered, "ks=http://192.168.1.100/kickstarts/192.168.1.100/server.ks");
        assert!(!rendered.contains("{{PXE_SERVER}}"));
    }

    #[test]
    fn test_render_to_kernel_arg_flag_without_value() {
        let arg = ImageKernelArg {
            id: Uuid::new_v4(),
            for_image: Uuid::new_v4(),
            _key: "quiet".to_string(),
            _value: None,
        };

        let rendered = arg.render_to_kernel_arg();
        assert_eq!(rendered, "quiet");

        let pxe_server = "192.168.1.100";
        let rendered_with_replacement = arg.render_to_kernel_arg_with_replacement(pxe_server);
        assert_eq!(rendered_with_replacement, "quiet");
    }

    #[test]
    fn test_render_to_kernel_arg_without_placeholder() {
        let arg = ImageKernelArg {
            id: Uuid::new_v4(),
            for_image: Uuid::new_v4(),
            _key: "console".to_string(),
            _value: Some("ttyS0,115200".to_string()),
        };

        let rendered = arg.render_to_kernel_arg();
        assert_eq!(rendered, "console=ttyS0,115200");

        let pxe_server = "192.168.1.100";
        let rendered_with_replacement = arg.render_to_kernel_arg_with_replacement(pxe_server);
        assert_eq!(rendered_with_replacement, "console=ttyS0,115200");
    }
}
