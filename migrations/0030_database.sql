-- A database is a service, and the difference is a row.
--
-- ## Why not a table of its own beside `service`
--
-- `service` and `replica` already carry deploying, reconciling,
-- observing, placing a copy on another node, reporting back and
-- eviction. A second kind of thing beside them would need every one of
-- those again, and the second copy is the one that drifts.
--
-- What a database has that a service does not is an *engine*: a
-- version, credentials, and an opinion about which copy accepts
-- writes. That is this table.
--
-- ## The kind is on the service, and it is not a hint
--
-- The deploy path reads it to decide what a container needs — a
-- volume, a memory ceiling, tuning arguments, a role. The console
-- reads it to decide which page to show: a managed database has no
-- image field and no environment editor, because the node writes both.
ALTER TABLE service ADD COLUMN kind TEXT NOT NULL DEFAULT 'container';

CREATE TABLE database (
    -- The service it is. One row per service, so the primary key is
    -- the service's: two `database` rows for one service would be two
    -- engines claiming one set of containers.
    service_id  TEXT PRIMARY KEY REFERENCES service (id) ON DELETE CASCADE,

    -- 'postgres'. The column exists before the second engine does
    -- because the alternative is finding every place that assumed.
    engine      TEXT NOT NULL,
    -- The **major** version, as the image tags spell it: '17'. Minor
    -- updates arrive by pulling the tag again. Changing the major is a
    -- data migration and is refused with that as the reason.
    version     TEXT NOT NULL,

    -- What the node generated. Held in the clear, like every registry
    -- credential on this node: the process that would read them is the
    -- one that has the file open, and encrypting them against a key
    -- kept beside them is a ritual rather than a defence.
    admin_user     TEXT NOT NULL,
    admin_password TEXT NOT NULL,
    database_name  TEXT NOT NULL,

    -- Replication logs in as its own role, never as the superuser. A
    -- node holding a read replica already has every byte on its disk,
    -- so the superuser password would buy it nothing it does not
    -- have — except the ability to *write* to the primary, which is
    -- exactly what a read-only copy must not be able to do.
    replication_user     TEXT NOT NULL,
    replication_password TEXT NOT NULL,

    -- Which slot accepts writes. A column rather than the constant 1,
    -- so that promoting a standby is one row rather than a migration.
    -- Nothing writes it yet; see `docs/databases.md`, "Open".
    primary_slot INTEGER NOT NULL DEFAULT 1,

    created_at INTEGER NOT NULL
);

-- The last octet of this copy's address on its project's bridge.
--
-- A connection string is written down — in an application's
-- environment, in somebody's notes — so the address behind it cannot
-- change every time the container is recreated. `host-local` hands out
-- the lowest free address, which is stable only while nothing else
-- churns.
--
-- So the project's /24 is split: `host-local` is bounded to the low
-- part and the high part is the node's own to hand out, which is the
-- same construction the two port ranges use. The two allocators cannot
-- collide because their ranges do not overlap, rather than because
-- both remember to consult the other's table. See
-- `runtime::network::RESERVED_HOSTS`.
--
-- Null is every replica that does not need one, which is all of them
-- until a database is created.
ALTER TABLE replica ADD COLUMN reserved_host INTEGER;
