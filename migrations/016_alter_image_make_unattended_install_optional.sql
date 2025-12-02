ALTER TABLE images
    ALTER COLUMN http_unattended_install_config_path DROP NOT NULL,
    ALTER COLUMN http_unattended_install_config_path DROP DEFAULT,
    ALTER COLUMN http_iso_path DROP NOT NULL,
    ALTER COLUMN http_iso_path DROP DEFAULT;
