-- Migration 015: update `images` and `flavors` to use native ENUM types and remove JSONB fields.
-- Safely handles existing VARCHAR values in migrated fields. Deletes several redundant fields and adds
-- more precise metadata fields on `flavor` table

CREATE TYPE arch AS ENUM ('x86_64', 'aarch64');
CREATE TYPE distro AS ENUM ('Ubuntu', 'Fedora', 'Alma', 'EVE');
CREATE TYPE storage_type AS ENUM ('ssd', 'hdd', 'hybrid');

ALTER TABLE images
    ADD COLUMN IF NOT EXISTS arch_new arch,
    ADD COLUMN IF NOT EXISTS distro_new distro,
    ADD COLUMN IF NOT EXISTS http_unattended_install_config_path TEXT,
    ADD COLUMN IF NOT EXISTS http_iso_path TEXT,
    ADD COLUMN IF NOT EXISTS tftp_kernel_path TEXT,
    ADD COLUMN IF NOT EXISTS tftp_initrd_paths TEXT[];

UPDATE images
SET arch_new = CASE
    WHEN arch IN ('x86_64', 'aarch64') THEN arch::arch
    ELSE 'x86_64'::arch  -- fallback
END
WHERE arch_new IS NULL;

UPDATE images
SET distro_new = CASE
    WHEN distro IN ('Ubuntu', 'Fedora', 'Alma', 'EVE') THEN distro::distro
    WHEN distro ILIKE '%ubuntu%' THEN 'Ubuntu'::distro
    WHEN distro ILIKE '%fedora%' THEN 'Fedora'::distro
    WHEN distro ILIKE '%alma%' THEN 'Alma'::distro
    WHEN distro ILIKE '%eve%' THEN 'EVE'::distro
    ELSE 'Ubuntu'::distro  -- fallback
END
WHERE distro_new IS NULL;

ALTER TABLE images
    DROP COLUMN IF EXISTS owner,
    DROP COLUMN IF EXISTS public,
    DROP COLUMN arch,
    DROP COLUMN distro;

ALTER TABLE images
    RENAME COLUMN arch_new TO arch;

ALTER TABLE images
    RENAME COLUMN distro_new TO distro;

ALTER TABLE images
    ALTER COLUMN arch SET NOT NULL,
    ALTER COLUMN arch SET DEFAULT 'x86_64'::arch,
    ALTER COLUMN distro SET NOT NULL,
    ALTER COLUMN distro SET DEFAULT 'Ubuntu'::distro;

ALTER TABLE images
    ALTER COLUMN version SET DEFAULT 'latest';

UPDATE images
SET http_unattended_install_config_path = '/'
WHERE http_unattended_install_config_path IS NULL;

UPDATE images
SET http_iso_path = '/'
WHERE http_iso_path IS NULL;

UPDATE images
SET tftp_kernel_path = '/'
WHERE tftp_kernel_path IS NULL;

UPDATE images
SET tftp_initrd_paths = '{}'
WHERE tftp_initrd_paths IS NULL;

ALTER TABLE images
    ALTER COLUMN http_unattended_install_config_path SET NOT NULL,
    ALTER COLUMN http_unattended_install_config_path SET DEFAULT '/',
    ALTER COLUMN http_iso_path SET NOT NULL,
    ALTER COLUMN http_iso_path SET DEFAULT '/',
    ALTER COLUMN tftp_kernel_path SET NOT NULL,
    ALTER COLUMN tftp_kernel_path SET DEFAULT '/',
    ALTER COLUMN tftp_initrd_paths SET NOT NULL,
    ALTER COLUMN tftp_initrd_paths SET DEFAULT '{}';

CREATE TABLE IF NOT EXISTS image_kernel_args (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    for_image UUID NOT NULL REFERENCES images(id) ON DELETE CASCADE,
    _key TEXT NOT NULL,
    _value TEXT
);

ALTER TABLE flavors
    ADD COLUMN IF NOT EXISTS description TEXT,
    ADD COLUMN IF NOT EXISTS cpu_frequency_mhz INTEGER,
    ADD COLUMN IF NOT EXISTS cpu_model TEXT,
    ADD COLUMN IF NOT EXISTS ram_bytes BIGINT,
    ADD COLUMN IF NOT EXISTS root_size_bytes BIGINT,
    ADD COLUMN IF NOT EXISTS disk_size_bytes BIGINT,
    ADD COLUMN IF NOT EXISTS storage_type storage_type,
    ADD COLUMN IF NOT EXISTS network_speed_mbps INTEGER,
    ADD COLUMN IF NOT EXISTS network_interfaces INTEGER,
    ADD COLUMN IF NOT EXISTS deleted BOOLEAN DEFAULT false;

ALTER TABLE flavors
    ALTER COLUMN brand DROP NOT NULL,
    ALTER COLUMN model DROP NOT NULL;

ALTER TABLE flavors
    ADD COLUMN IF NOT EXISTS cpu_count_new INTEGER;

UPDATE flavors
SET cpu_count_new = (
    CASE
        WHEN jsonb_typeof(cpu_count) = 'number' THEN (cpu_count::text)::integer
        WHEN jsonb_typeof(cpu_count) = 'string' THEN (cpu_count->>0)::integer
        ELSE NULL
    END
)
WHERE cpu_count_new IS NULL AND cpu_count IS NOT NULL;

ALTER TABLE flavors
    DROP COLUMN cpu_count;

ALTER TABLE flavors
    RENAME COLUMN cpu_count_new TO cpu_count;

ALTER TABLE flavors
    DROP COLUMN IF EXISTS ram,
    DROP COLUMN IF EXISTS root_size,
    DROP COLUMN IF EXISTS disk_size,
    DROP COLUMN IF EXISTS swap_size,
    DROP COLUMN IF EXISTS public;

ALTER TABLE flavors
    ALTER COLUMN name TYPE VARCHAR(255),
    ALTER COLUMN brand TYPE VARCHAR(255),
    ALTER COLUMN model TYPE VARCHAR(255);

ALTER TABLE flavors
    ADD COLUMN IF NOT EXISTS arch_new arch;

UPDATE flavors
SET arch_new = CASE
    WHEN arch IN ('x86_64', 'aarch64') THEN arch::arch
    WHEN arch ILIKE '%x86%' OR arch ILIKE '%amd64%' THEN 'x86_64'::arch
    WHEN arch ILIKE '%aarch%' OR arch ILIKE '%arm64%' THEN 'aarch64'::arch
    ELSE 'x86_64'::arch  -- default
END
WHERE arch_new IS NULL;

ALTER TABLE flavors
    DROP COLUMN arch;

ALTER TABLE flavors
    RENAME COLUMN arch_new TO arch;

ALTER TABLE flavors
    ALTER COLUMN arch SET NOT NULL,
    ALTER COLUMN arch SET DEFAULT 'x86_64'::arch;
