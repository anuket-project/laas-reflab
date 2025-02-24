-- Add a nullable bmc_vlan_id column to the host_ports table
ALTER TABLE host_ports
ADD COLUMN bmc_vlan_id smallint NULL;

