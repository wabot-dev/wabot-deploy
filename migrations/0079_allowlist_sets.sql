-- Lines a project keeps once, under a name, for the services that use the
-- list: an office, a VPN, a CDN's ranges. What a service admits is its own
-- lines and every line of each list it uses.
--
-- Its own tables rather than rows in `allowlist`: a list is not a door, and
-- nothing guards it until a service uses it.
CREATE TABLE allowlist_set (
    id         TEXT    PRIMARY KEY,
    project_id TEXT    NOT NULL REFERENCES project (id) ON DELETE CASCADE,
    name       TEXT    NOT NULL,
    created_at INTEGER NOT NULL,
    UNIQUE (project_id, name)
);

CREATE TABLE allowlist_set_entry (
    set_id     TEXT    NOT NULL REFERENCES allowlist_set (id) ON DELETE CASCADE,
    network    TEXT    NOT NULL,
    note       TEXT,
    created_at INTEGER NOT NULL,
    PRIMARY KEY (set_id, network)
);

CREATE TABLE service_allowlist_set (
    service_id TEXT NOT NULL REFERENCES service (id) ON DELETE CASCADE,
    set_id     TEXT NOT NULL REFERENCES allowlist_set (id) ON DELETE CASCADE,
    PRIMARY KEY (service_id, set_id)
);
