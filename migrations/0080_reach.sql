-- What a project runs on other nodes, as its owner last said, so a
-- container here reaches a service with no copy here by its name. Kept
-- whole, as the errand carried it: the owner decides what is in it, and a
-- row per copy would be a second set of the owner's books to keep in step.
CREATE TABLE project_directory (
    project_id TEXT    PRIMARY KEY REFERENCES project (id) ON DELETE CASCADE,
    body       TEXT    NOT NULL,
    updated_at INTEGER NOT NULL
);

-- The address in 10.42.255.0/24 a name with no copy here resolves to. Kept
-- rather than recomputed: a client that resolved it once may hold on to it
-- for as long as it runs, so it must not move when another name comes or
-- goes.
CREATE TABLE stand_in (
    project_id TEXT    NOT NULL REFERENCES project (id) ON DELETE CASCADE,
    name       TEXT    NOT NULL,
    host       INTEGER NOT NULL,
    PRIMARY KEY (project_id, name),
    UNIQUE (project_id, host)
);
