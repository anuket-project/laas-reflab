ALTER TABLE switchports
ADD CONSTRAINT switchports_on_switch_name_unique
UNIQUE (for_switch, name);
