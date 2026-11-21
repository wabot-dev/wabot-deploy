-- An address a name on the list resolved to, open until the answer that
-- named it is out of date. Kept rather than held in memory, so a node that
-- restarts does not close what a program cached before it did. Port 0 is
-- every port.
CREATE TABLE egress_learned (
    project_id TEXT    NOT NULL REFERENCES project (id) ON DELETE CASCADE,
    address    TEXT    NOT NULL,
    port       INTEGER NOT NULL,
    name       TEXT    NOT NULL,
    expires_at INTEGER NOT NULL,
    PRIMARY KEY (project_id, address, port)
);

-- A name a container asked for and the list does not hold, so the
-- project's page can say why something failed to resolve.
CREATE TABLE egress_refused (
    project_id TEXT    NOT NULL REFERENCES project (id) ON DELETE CASCADE,
    name       TEXT    NOT NULL,
    times      INTEGER NOT NULL,
    last_at    INTEGER NOT NULL,
    PRIMARY KEY (project_id, name)
);
