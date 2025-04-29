BEGIN;

ALTER TABLE host_ports ADD COLUMN mac_native macaddr;

UPDATE host_ports
SET mac_native = (
    SELECT (
      string_agg(
        lpad(to_hex((elem::text)::int), 2, '0'),   -- e.g. 28 → '1c'
        ':'                                        -- join with colons
        ORDER BY ordinality
      )
    )::macaddr
    FROM jsonb_array_elements_text(mac) WITH ORDINALITY arr(elem, ordinality)
);

ALTER TABLE host_ports
  DROP COLUMN mac;

ALTER TABLE host_ports RENAME COLUMN mac_native TO mac;

COMMIT;
