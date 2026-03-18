CREATE TABLE IF NOT EXISTS flavor_commands (
    for_flavor UUID NOT NULL,
    for_image UUID NOT NULL,
    commands TEXT[] NOT NULL,

    PRIMARY KEY (for_flavor, for_image),

    CONSTRAINT fk_flavor 
        FOREIGN KEY (for_flavor) 
        REFERENCES flavors(id) 
        ON DELETE CASCADE,

    CONSTRAINT fk_image 
        FOREIGN KEY (for_image) 
        REFERENCES images(id) 
        ON DELETE CASCADE
);