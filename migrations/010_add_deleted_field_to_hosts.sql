ALTER TABLE hosts ADD COLUMN deleted boolean NOT NULL DEFAULT false;

CREATE INDEX hosts_deleted_idx ON hosts (deleted);

