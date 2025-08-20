ALTER TABLE images ADD distro varchar NOT NULL DEFAULT 'Ubuntu';
ALTER TABLE images ADD version varchar NOT NULL DEFAULT '22.04';
ALTER TABLE images ADD arch varchar NOT NULL DEFAULT 'x86_64';
