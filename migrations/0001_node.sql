-- The node's own bookkeeping.
--
-- Everything the *platform* stores — projects, services, deployments —
-- arrives later and goes in the framework's document store, which
-- creates its own tables. These two are relational because they are
-- small, fixed, and read by `doctor` as much as by the daemon.

-- The install ledger. One row per step, so re-running `install`
-- converges rather than repeating, and a run that died halfway can say
-- where it stopped.
CREATE TABLE node_state (
    step       TEXT PRIMARY KEY,
    status     TEXT NOT NULL,
    detail     TEXT,
    updated_at INTEGER NOT NULL
);

-- A `setting` table for one-off values (the node id, the bootstrap
-- admin token) belongs here too, and is deliberately absent: nothing
-- writes one yet. It arrives in the migration that adds whatever
-- needs it, which is what the migration runner is for.
