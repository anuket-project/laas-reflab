ALTER TABLE host_ports ALTER COLUMN switchport DROP NOT NULL;

ALTER TABLE host_ports
  DROP COLUMN is_a;

