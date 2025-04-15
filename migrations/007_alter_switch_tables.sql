ALTER TABLE switches
    DROP COLUMN IF EXISTS management_vlans,
    DROP COLUMN IF EXISTS ipmi_vlan,
    DROP COLUMN IF EXISTS public_vlans;

ALTER TABLE switch_os
  DROP COLUMN IF EXISTS version;
