BEGIN;

ALTER TABLE provision_log_events
ALTER COLUMN time TYPE timestamptz USING time AT TIME ZONE 'UTC';

ALTER TABLE allocations
ALTER COLUMN started TYPE timestamptz USING started AT TIME ZONE 'UTC';

ALTER TABLE allocations
ALTER COLUMN ended TYPE timestamptz USING ended AT TIME ZONE 'UTC';

COMMIT;
