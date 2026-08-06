-- Projects and the services inside them.

-- A project is a workspace: a name, and the services under it. It owns
-- no containerd namespace of its own — one namespace for the whole
-- node, containers labelled with their project.
--
-- The reason is content. containerd namespaces scope image blobs as
-- well as metadata, so a namespace per project means a second copy of
-- every layer two projects share, and it breaks the embedded registry
-- that shares the content store. What a namespace does *not* give is
-- network, resource or security isolation — so "a project cannot see
-- another's containers" is this node's API to enforce, which is where
-- it belongs anyway.
CREATE TABLE project (
    id         TEXT PRIMARY KEY,
    name       TEXT    NOT NULL,
    -- URL-safe, and the prefix of every container and hostname the
    -- project owns.
    slug       TEXT    NOT NULL,
    created_at INTEGER NOT NULL
);

CREATE UNIQUE INDEX project_slug ON project (slug);

CREATE TABLE service (
    id         TEXT PRIMARY KEY,
    project_id TEXT    NOT NULL REFERENCES project (id) ON DELETE CASCADE,
    name       TEXT    NOT NULL,
    slug       TEXT    NOT NULL,
    image      TEXT    NOT NULL,

    -- What the application listens on inside the container. Read from
    -- the image's ExposedPorts when the operator does not say.
    container_port INTEGER,
    -- What the node gave it on the host. Containers share the host's
    -- network for now, so two services cannot hold the same one and
    -- the node is what allocates.
    host_port      INTEGER,

    -- JSON object. A table would be tidier and this is read and written
    -- whole, always, by one writer.
    env TEXT NOT NULL DEFAULT '{}',

    -- What the operator asked for: 'running' or 'stopped'. Distinct
    -- from what containerd reports, which is what *is*. A service can
    -- be desired-running and crashed, and conflating the two loses the
    -- only thing that says which.
    desired_state TEXT NOT NULL DEFAULT 'running',

    -- Set when a deployment fails, cleared when one succeeds.
    last_error TEXT,

    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

-- Unique per project, not globally: two projects may each have an
-- `api`, and being able to is the point of projects.
CREATE UNIQUE INDEX service_slug ON service (project_id, slug);
CREATE INDEX service_project ON service (project_id);

-- The host port each service holds, so allocation can find a free one
-- without scanning every row.
CREATE UNIQUE INDEX service_host_port ON service (host_port) WHERE host_port IS NOT NULL;
