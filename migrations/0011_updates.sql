-- One row per attempt to install a release.
--
-- The row is the only thing that survives the step it exists to
-- report: applying an update ends with the process being replaced, so
-- "did it work" has to be a question the *next* process can answer.
-- It marks itself `restarting`, and the node that comes back reads the
-- row, compares versions and settles it.
CREATE TABLE update_run (
    id           TEXT PRIMARY KEY,
    -- What was running when this started, and what it is going to.
    from_version TEXT NOT NULL,
    to_version   TEXT NOT NULL,
    tag          TEXT NOT NULL,
    -- 'running' | 'restarting' | 'done' | 'failed'
    status       TEXT NOT NULL,
    -- The step in progress, for a page somebody is watching.
    step         TEXT,
    -- Why it failed, or what it did.
    detail       TEXT,
    -- The copy of the database taken before the new binary could
    -- migrate it. A path, not the bytes: this table lives in the file
    -- being backed up.
    backup_path  TEXT,
    -- Who asked. An update is the most consequential button here.
    account_id   TEXT REFERENCES account (id) ON DELETE SET NULL,
    started_at   INTEGER NOT NULL,
    finished_at  INTEGER
);

CREATE INDEX update_run_started ON update_run (started_at DESC);
