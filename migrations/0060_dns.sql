-- Zones this node answers DNS for.
--
-- A zone is administered from the node where it was created, like a
-- service: `authority_id` is NULL there, and on a node it was sent to it
-- names the sender, which is what makes the copy read-only and evictable.
-- One row per name on a node, whoever it belongs to — two zones for one
-- name would be two answers to one question.
CREATE TABLE dns_zone (
    id                    TEXT PRIMARY KEY,
    name                  TEXT NOT NULL UNIQUE,
    tenant_id             TEXT REFERENCES tenant (id) ON DELETE CASCADE,
    authority_id          TEXT,
    serial                INTEGER NOT NULL,
    default_ttl           INTEGER NOT NULL DEFAULT 300,
    delegation_status     TEXT,
    delegation_note       TEXT,
    delegation_checked_at INTEGER,
    created_at            INTEGER NOT NULL,
    updated_at            INTEGER NOT NULL
);

CREATE TABLE dns_record (
    id         TEXT PRIMARY KEY,
    zone_id    TEXT NOT NULL REFERENCES dns_zone (id) ON DELETE CASCADE,
    name       TEXT NOT NULL,
    kind       TEXT NOT NULL,
    value      TEXT NOT NULL,
    priority   INTEGER,
    ttl        INTEGER NOT NULL,
    created_at INTEGER NOT NULL
);

CREATE INDEX dns_record_zone ON dns_record (zone_id);

-- Which nodes answer for a zone, in the order they are listed as its name
-- servers.
CREATE TABLE dns_zone_server (
    zone_id  TEXT NOT NULL REFERENCES dns_zone (id) ON DELETE CASCADE,
    node_id  TEXT NOT NULL,
    position INTEGER NOT NULL,
    PRIMARY KEY (zone_id, node_id)
);
