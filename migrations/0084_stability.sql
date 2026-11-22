-- What a copy did, ten minutes at a time, on the node that runs it
-- (`docs/stability.md`).
--
-- Keyed by the container's name rather than the replica row: the name is
-- the same on the node that placed the copy and on the node running it,
-- which is what a far page asks by, and it outlives a row that an
-- eviction removed and a placement made again.
--
-- `samples` counts the readings taken while the service was meant to run
-- and `running` the ones that found it up, so a copy that was down and a
-- node that was not measuring are two different things on the page: the
-- first is a row with fewer running than samples, the second is no row.
CREATE TABLE copy_metric (
    container      TEXT    NOT NULL,
    bucket         INTEGER NOT NULL,
    samples        INTEGER NOT NULL,
    running        INTEGER NOT NULL,
    memory_max     INTEGER,
    memory_limit   INTEGER,
    cpu_avg        INTEGER,
    cpu_max        INTEGER,
    cpu_limit      INTEGER,
    throttled      INTEGER,
    received       INTEGER,
    sent           INTEGER,
    read           INTEGER,
    written        INTEGER,
    disk           INTEGER,
    disk_partial   INTEGER NOT NULL DEFAULT 0,
    disk_limit     INTEGER,
    PRIMARY KEY (container, bucket)
) WITHOUT ROWID;

-- What happened to a service, written where it happened: a deployment, a
-- stop and a backup on the node that owns the service, a crash on the node
-- whose copy it was. `detail` is never translated — an image reference, a
-- process's last line, a destination's refusal.
CREATE TABLE service_event (
    id         TEXT    PRIMARY KEY,
    service_id TEXT    NOT NULL REFERENCES service (id) ON DELETE CASCADE,
    container  TEXT,
    kind       TEXT    NOT NULL,
    at         INTEGER NOT NULL,
    until      INTEGER,
    code       INTEGER,
    detail     TEXT
);

CREATE INDEX service_event_at ON service_event (service_id, at);
