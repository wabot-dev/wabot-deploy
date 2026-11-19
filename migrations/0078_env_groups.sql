-- Variables a project keeps once, under a name, for the services that join
-- the group: an SMTP account or an error tracker's key several services
-- read. A service's own variables win over a group's, and two groups one
-- service joins never name the same variable — the commands refuse it, so
-- what a container gets has one answer.
--
-- `env` is a JSON object, as on `service`: read and written whole.
CREATE TABLE env_group (
    id         TEXT    PRIMARY KEY,
    project_id TEXT    NOT NULL REFERENCES project (id) ON DELETE CASCADE,
    name       TEXT    NOT NULL,
    env        TEXT    NOT NULL DEFAULT '{}',
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    UNIQUE (project_id, name)
);

CREATE TABLE service_env_group (
    service_id TEXT NOT NULL REFERENCES service (id) ON DELETE CASCADE,
    group_id   TEXT NOT NULL REFERENCES env_group (id) ON DELETE CASCADE,
    PRIMARY KEY (service_id, group_id)
);
