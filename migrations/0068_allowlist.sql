-- Who may come in through each of the node's doors: an address or a range
-- per row. **No rows is a closed door** — "everyone" is the two rows
-- 0.0.0.0/0 and ::/0, written down by somebody.
--
-- `subject` is the organisation, project or service a door belongs to,
-- and empty for the node's own two: its console and the registry under
-- its name.
CREATE TABLE allowlist (
    door       TEXT    NOT NULL,
    subject    TEXT    NOT NULL DEFAULT '',
    network    TEXT    NOT NULL,
    note       TEXT,
    created_at INTEGER NOT NULL,
    PRIMARY KEY (door, subject, network)
);

-- Every door that exists already is opened to everyone, which is what it
-- was. Without this an upgrade would close the console, every
-- organisation's, every registry and every service at once. A new node
-- gets the same for its console and its registry, or `/setup` would be
-- the first page nobody can reach.
WITH everyone (network) AS (VALUES ('0.0.0.0/0'), ('::/0'))
INSERT INTO allowlist (door, subject, network, created_at)
SELECT 'console', '', network, 0 FROM everyone
UNION ALL SELECT 'registry', '', network, 0 FROM everyone
UNION ALL SELECT 'organisation', tenant.id, network, 0 FROM tenant, everyone
UNION ALL SELECT 'project_registry', project.id, network, 0
    FROM project, everyone WHERE project.origin_node_id IS NULL
UNION ALL SELECT 'service', service.id, network, 0
    FROM service, everyone WHERE service.origin_node_id IS NULL;

-- A list goes with what it belongs to. Triggers rather than a delete
-- beside each caller: a project takes its services by `ON DELETE CASCADE`,
-- which no caller sees, and a cascaded delete fires these.
CREATE TRIGGER allowlist_of_a_service AFTER DELETE ON service BEGIN
    DELETE FROM allowlist WHERE door = 'service' AND subject = OLD.id;
END;
CREATE TRIGGER allowlist_of_a_project AFTER DELETE ON project BEGIN
    DELETE FROM allowlist WHERE door = 'project_registry' AND subject = OLD.id;
END;
CREATE TRIGGER allowlist_of_an_organisation AFTER DELETE ON tenant BEGIN
    DELETE FROM allowlist WHERE door = 'organisation' AND subject = OLD.id;
END;

-- What the owner of a name said about who may reach it, on a route this
-- node answers for another node: the same lines, comma-separated, and ''
-- for nobody. NULL is an owner that said nothing — one older than
-- allowlists, whose names are served as they were.
ALTER TABLE route ADD COLUMN allowed TEXT;
