-- A replica: one running copy of a service, on one node.
--
-- ## Why the service stopped being the unit
--
-- A service was one container, and its id was derived from the service
-- name — so there could only ever be one of it, here. The product's
-- goal is that the node which *created* a service decides how many
-- copies run and where each of them goes, including several on one
-- machine. That makes the placement the thing with an identity, and the
-- service the thing that describes what to place.
--
-- ## `node_id` is null for here
--
-- Rather than this node's own id. Two reasons, and the second is the
-- one that matters: a migration cannot know the id — it is minted at
-- install and lives in another table — so a backfill would have to
-- guess or be deferred to code. And "here" is the answer to a different
-- question than "which node": it is the absence of a placement
-- elsewhere, exactly the way `endpoint` being null is what makes a node
-- private.
--
-- ## Every existing service becomes one replica, here
--
-- Carrying its address across, so a node that upgrades has the same
-- containers it had, described the new way. What it must *not* do is
-- rename them — see `services::Service::container_id`.
CREATE TABLE replica (
    id         TEXT PRIMARY KEY,
    service_id TEXT    NOT NULL REFERENCES service (id) ON DELETE CASCADE,
    -- Which node runs it. Null is this one.
    node_id    TEXT,
    -- Its number within the service, from 1. Not called `index`, which
    -- SQLite reserves, and `slot` is the better word anyway: it is a
    -- position that can be emptied and filled rather than a count.
    slot       INTEGER NOT NULL,
    -- The container's address on its project's bridge, while it runs.
    -- Per replica now: two copies of one service are two containers
    -- with two addresses, which is the whole point of having two.
    address    TEXT,
    -- Why this one is not running, when it is not. Per replica for the
    -- same reason: one copy failing to pull is not the service failing.
    last_error TEXT,
    -- Set when the node running it threw it out. The node that placed
    -- it stops asking — a danger zone that the origin undid would not
    -- be one — and the operator there decides what happens next.
    evicted_at INTEGER,
    created_at INTEGER NOT NULL
);

-- One replica per slot per service. Two rows claiming slot 2 would be
-- two containers with one id on whichever node they landed on.
CREATE UNIQUE INDEX replica_slot ON replica (service_id, slot);
CREATE INDEX replica_node ON replica (node_id);

-- What every service already has, described the new way: one replica,
-- here, holding the address the service row was carrying.
INSERT INTO replica ("id", "service_id", "node_id", "slot", "address", "created_at")
SELECT 'rp-' || "id", "id", NULL, 1, "address", unixepoch() * 1000 FROM service;
