CREATE TABLE IF NOT EXISTS tascii_database_objects (
  id uuid PRIMARY KEY NOT NULL,
  v jsonb
);

CREATE TABLE IF NOT EXISTS tascii_runtime_tasks (
  id uuid PRIMARY KEY NOT NULL,
  proto jsonb NOT NULL,
  result jsonb NOT NULL,
  context jsonb NOT NULL,
  depends_on uuid[] NOT NULL,
  waiting_for uuid[] NOT NULL,
  depends_for uuid[] NOT NULL,
  state jsonb NOT NULL
);
