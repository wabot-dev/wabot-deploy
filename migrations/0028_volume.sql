-- Storage that outlives the container.
--
-- ## Why a container had none
--
-- `containers::run` removes whatever is under the id before it starts,
-- and removing a container removes its snapshot with it — so everything
-- a container wrote is gone at the next deployment. That is correct for
-- a stateless service, where starting from the image is the whole
-- point, and it is total loss for anything that keeps data.
--
-- ## A row per mount point, a directory per copy
--
-- What to mount and where belongs to the **service**: every copy runs
-- the same image with the same layout, so a second row per replica
-- would be the same answer written n times, able to disagree.
--
-- The bytes belong to the **replica**. Two copies of a database on one
-- node are two databases, and a directory they shared would be two
-- servers writing one data directory — which corrupts it in seconds,
-- not eventually.
--
-- So there is no directory in this table. It is derived, the way a
-- container id is and for the same reason: what cleans up after a crash
-- starts from the rows and asks the disk what is there. See
-- `platform::volumes::directory`.
CREATE TABLE volume (
    id         TEXT PRIMARY KEY,
    service_id TEXT NOT NULL REFERENCES service (id) ON DELETE CASCADE,

    -- What an operator calls it, and the last component of the
    -- directory on the node. A slug, so `ls` on a node's volumes reads
    -- as `<project>.<service>.<slot>/data`.
    name       TEXT NOT NULL,

    -- Where it is mounted inside the container. Absolute, checked
    -- against the handful of paths a container cannot survive losing.
    path       TEXT NOT NULL,

    created_at INTEGER NOT NULL
);

-- Neither can repeat within a service: two rows on one path would be
-- two mounts racing for one destination, and two on one name would be
-- one directory serving both.
CREATE UNIQUE INDEX volume_service_name ON volume (service_id, name);
CREATE UNIQUE INDEX volume_service_path ON volume (service_id, path);
CREATE INDEX volume_service ON volume (service_id);
