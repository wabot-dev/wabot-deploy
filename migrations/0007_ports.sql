-- What a service exposes, and how.
--
-- A service had one optional port, which conflated three unrelated
-- questions: what does the process listen on, is that reachable from
-- outside the node, and does it answer HTTPS at a hostname. Most
-- services expose nothing; some expose a database port to the world;
-- some serve a site; a few do two of those at once.
--
-- One row per port, and the two nullable columns say which of the
-- three it is. Both NULL is the common case: the port exists, other
-- containers in the project can reach it, nothing outside can.
CREATE TABLE port (
    id             TEXT PRIMARY KEY,
    service_id     TEXT    NOT NULL REFERENCES service (id) ON DELETE CASCADE,
    -- What the process listens on inside the container.
    container_port INTEGER NOT NULL,

    -- The port on the node's public address, when this one is
    -- published as raw TCP. NULL means it is not.
    --
    -- Held even while the service is stopped: a published port is an
    -- address somebody wrote down, and handing it to another service
    -- the moment this one restarts is worse than holding it.
    host_port      INTEGER,

    -- The hostname this port answers HTTPS on. NULL means it does not.
    --
    -- Verified to resolve to this node before it is stored: a route
    -- for a name that points somewhere else is a certificate request
    -- that fails and a page that never loads.
    hostname       TEXT,

    created_at     INTEGER NOT NULL
);

-- A service cannot declare the same port twice.
CREATE UNIQUE INDEX port_service_container ON port (service_id, container_port);

-- Two services cannot hold the same host port or the same hostname.
-- Partial, so the unpublished and un-hosted rows — the majority — do
-- not collide with each other on NULL.
CREATE UNIQUE INDEX port_host_port ON port (host_port) WHERE host_port IS NOT NULL;
CREATE UNIQUE INDEX port_hostname  ON port (hostname)  WHERE hostname  IS NOT NULL;

-- Carry over what the old single-port column held. It described the
-- port inside the container and nothing else, which is exactly a row
-- with both nullable columns empty.
INSERT INTO port (id, service_id, container_port, created_at)
SELECT 'prt-' || "id", "id", "container_port", "updated_at"
  FROM service WHERE "container_port" IS NOT NULL;

-- The index has to go before the column it covers.
DROP INDEX IF EXISTS service_host_port;
ALTER TABLE service DROP COLUMN container_port;
ALTER TABLE service DROP COLUMN host_port;
