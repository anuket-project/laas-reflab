CREATE TABLE IF NOT EXISTS templates (
  id uuid PRIMARY KEY NOT NULL,
  owner VARCHAR,
  name VARCHAR NOT NULL,
  deleted boolean NOT NULL,
  "public" boolean NOT NULL,
  description VARCHAR NOT NULL,
  networks uuid[] NOT NULL,
  hosts jsonb NOT NULL,
  lab uuid
);

CREATE TABLE IF NOT EXISTS resource_handles (
  id uuid PRIMARY KEY NOT NULL,
  tracks_resource uuid UNIQUE NOT NULL,
  tracks_resource_type VARCHAR NOT NULL,
  lab uuid NOT NULL
);

CREATE TABLE IF NOT EXISTS switch_os (
  id uuid PRIMARY KEY NOT NULL,
  os_type VARCHAR NOT NULL,
  version VARCHAR NOT NULL
);

CREATE TABLE IF NOT EXISTS switches (
  id uuid PRIMARY KEY NOT NULL,
  name VARCHAR NOT NULL,
  ip VARCHAR NOT NULL,
  switch_user VARCHAR NOT NULL,
  switch_pass VARCHAR NOT NULL,
  switch_os uuid,
  management_vlans smallint[] NOT NULL,
  ipmi_vlan smallint NOT NULL,
  public_vlans smallint[] NOT NULL
);

CREATE TABLE IF NOT EXISTS switchports (
  id uuid PRIMARY KEY NOT NULL,
  for_switch uuid NOT NULL,
  name VARCHAR NOT NULL
);

CREATE TABLE IF NOT EXISTS vlans (
  id uuid PRIMARY KEY NOT NULL,
  vlan_id smallint NOT NULL,
  public_config jsonb
);

CREATE TABLE IF NOT EXISTS lab_statuses (
  id uuid PRIMARY KEY NOT NULL,
  for_lab uuid NOT NULL,
  time timestamp NOT NULL,
  expected_next_event_time jsonb NOT NULL,
  status jsonb NOT NULL,
  headline VARCHAR,
  subline VARCHAR
);

CREATE TABLE IF NOT EXISTS labs (
  id uuid PRIMARY KEY NOT NULL,
  name VARCHAR NOT NULL,
  location VARCHAR NOT NULL,
  email VARCHAR NOT NULL,
  phone VARCHAR NOT NULL,
  is_dynamic boolean NOT NULL
);

CREATE TABLE IF NOT EXISTS network_assignments (
  id uuid PRIMARY KEY NOT NULL,
  networks jsonb NOT NULL
);

CREATE TABLE IF NOT EXISTS networks (
  id uuid PRIMARY KEY NOT NULL,
  name VARCHAR NOT NULL,
  "public" boolean NOT NULL
);

CREATE TABLE IF NOT EXISTS ci_files (
  id uuid PRIMARY KEY NOT NULL,
  data VARCHAR NOT NULL,
  priority smallint NOT NULL
);

CREATE TABLE IF NOT EXISTS host_actions (
  id uuid PRIMARY KEY NOT NULL,
  for_host uuid NOT NULL,
  in_tascii VARCHAR NOT NULL,
  is_complete boolean NOT NULL
);

CREATE TABLE IF NOT EXISTS images (
  id uuid PRIMARY KEY NOT NULL,
  owner VARCHAR NOT NULL,
  name VARCHAR NOT NULL,
  deleted boolean NOT NULL,
  cobbler_name VARCHAR NOT NULL,
  "public" boolean NOT NULL,
  flavors uuid[] NOT NULL
);

CREATE TABLE IF NOT EXISTS aggregates (
  id uuid PRIMARY KEY NOT NULL,
  deleted boolean NOT NULL,
  users VARCHAR[] NOT NULL,
  vlans uuid NOT NULL,
  metadata jsonb NOT NULL,
  lifecycle_state jsonb NOT NULL,
  template uuid NOT NULL,
  configuration jsonb NOT NULL,
  lab uuid NOT NULL,
  CONSTRAINT aggregates_template_fkey FOREIGN KEY (template) REFERENCES templates (id) ON DELETE RESTRICT,
  CONSTRAINT aggregates_vlans_fkey FOREIGN KEY (vlans) REFERENCES network_assignments (id) ON DELETE RESTRICT
);

CREATE TABLE IF NOT EXISTS allocations (
  id uuid PRIMARY KEY NOT NULL,
  for_resource uuid NOT NULL,
  for_aggregate uuid,
  started timestamp NOT NULL,
  ended timestamp,
  reason_started VARCHAR NOT NULL,
  reason_ended VARCHAR,
  UNIQUE (for_resource, ended),
  CONSTRAINT allocations_for_aggregate_fkey FOREIGN KEY (for_aggregate) REFERENCES aggregates (id) ON DELETE RESTRICT,
  CONSTRAINT allocations_for_resource_fkey FOREIGN KEY (for_resource) REFERENCES resource_handles (id) ON DELETE RESTRICT
);

CREATE TABLE IF NOT EXISTS flavors (
  id uuid PRIMARY KEY NOT NULL,
  arch VARCHAR NOT NULL,
  name VARCHAR(1000) UNIQUE NOT NULL,
  "public" boolean NOT NULL,
  cpu_count jsonb NOT NULL,
  ram jsonb NOT NULL,
  root_size jsonb NOT NULL,
  disk_size jsonb NOT NULL,
  swap_size jsonb NOT NULL,
  brand VARCHAR(1000) NOT NULL,
  model VARCHAR(1000) NOT NULL,
  CONSTRAINT flavor_name_index UNIQUE (name)
);

CREATE TABLE IF NOT EXISTS extra_flavor_info (
  id uuid PRIMARY KEY NOT NULL,
  for_flavor uuid NOT NULL,
  extra_trait VARCHAR NOT NULL,
  key VARCHAR NOT NULL,
  value VARCHAR NOT NULL,
  CONSTRAINT extra_flavor_info_for_flavor_fkey FOREIGN KEY (for_flavor) REFERENCES flavors (id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS hosts (
  id uuid PRIMARY KEY NOT NULL,
  server_name VARCHAR(1000) UNIQUE NOT NULL,
  arch jsonb NOT NULL,
  flavor uuid NOT NULL,
  serial VARCHAR(1000) NOT NULL,
  ipmi_fqdn VARCHAR(1000) NOT NULL,
  iol_id VARCHAR(1000) NOT NULL,
  ipmi_mac macaddr NOT NULL,
  ipmi_user VARCHAR(1000) NOT NULL,
  ipmi_pass VARCHAR(1000) NOT NULL,
  projects jsonb NOT NULL,
  fqdn VARCHAR NOT NULL,
  sda_uefi_device VARCHAR(100),
  CONSTRAINT hosts_flavor_fkey FOREIGN KEY (flavor) REFERENCES flavors (id)
);

CREATE TABLE IF NOT EXISTS host_ports (
  id uuid PRIMARY KEY NOT NULL,
  on_host uuid NOT NULL,
  switchport uuid NOT NULL,
  name VARCHAR NOT NULL,
  speed jsonb NOT NULL,
  mac jsonb NOT NULL,
  switch VARCHAR NOT NULL,
  bus_addr VARCHAR NOT NULL,
  is_a uuid NOT NULL,
  CONSTRAINT host_ports_on_host_fkey FOREIGN KEY (on_host) REFERENCES hosts (id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS instances (
  id uuid PRIMARY KEY NOT NULL,
  within_template uuid NOT NULL,
  aggregate uuid NOT NULL,
  config jsonb NOT NULL,
  network_data uuid NOT NULL,
  linked_host uuid,
  metadata jsonb NOT NULL,
  CONSTRAINT instances_aggregate_fkey FOREIGN KEY (aggregate) REFERENCES aggregates (id) ON DELETE RESTRICT,
  CONSTRAINT instances_linked_host_fkey FOREIGN KEY (linked_host) REFERENCES hosts (id) ON DELETE RESTRICT,
  CONSTRAINT instances_network_data_fkey FOREIGN KEY (network_data) REFERENCES network_assignments (id) ON DELETE RESTRICT,
  CONSTRAINT instances_within_template_fkey FOREIGN KEY (within_template) REFERENCES templates (id) ON DELETE RESTRICT
);

CREATE TABLE IF NOT EXISTS interface_flavors (
  id uuid PRIMARY KEY NOT NULL,
  on_flavor uuid NOT NULL,
  name VARCHAR NOT NULL,
  speed jsonb NOT NULL,
  cardtype jsonb NOT NULL,
  CONSTRAINT interface_flavors_on_flavor_fkey FOREIGN KEY (on_flavor) REFERENCES flavors (id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS provision_log_events (
  id uuid PRIMARY KEY NOT NULL,
  instance uuid NOT NULL,
  time timestamp NOT NULL,
  prov_status jsonb NOT NULL,
  sentiment jsonb,
  CONSTRAINT provision_log_events_instance_fkey FOREIGN KEY (instance) REFERENCES instances (id) ON DELETE CASCADE
);



CREATE TABLE IF NOT EXISTS vpn_tokens (
  id uuid PRIMARY KEY NOT NULL,
  username VARCHAR NOT NULL,
  project VARCHAR NOT NULL,
  CONSTRAINT vpn_tokens_owner_index UNIQUE (username)
);

