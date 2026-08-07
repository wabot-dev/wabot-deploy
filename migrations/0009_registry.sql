-- Receiving images, and being able to go back.
--
-- Three things that arrive together because they are one workflow:
-- somebody pushes an image, that becomes a release, and a release can
-- be rolled back to.

-- What a `docker login` uses as a password.
--
-- Scoped to one project, revocable, and not anybody's password —
-- which is the point. A CI machine holds this in a config file, and
-- the worst a leaked one does is push images to one project.
--
-- Stored hashed, like every other token here.
CREATE TABLE push_token (
    id           TEXT PRIMARY KEY,
    project_id   TEXT    NOT NULL REFERENCES project (id) ON DELETE CASCADE,
    token_hash   TEXT    NOT NULL,
    -- What it is for, so a list of five tokens is readable.
    name         TEXT    NOT NULL,
    created_by   TEXT    REFERENCES account (id) ON DELETE SET NULL,
    created_at   INTEGER NOT NULL,
    -- Answers "is this one still in use" before somebody revokes it.
    last_used_at INTEGER
);

CREATE UNIQUE INDEX push_token_hash ON push_token (token_hash);
CREATE INDEX push_token_project ON push_token (project_id);

-- One image, at one moment, for one service.
--
-- The digest is what makes this a release rather than a note: a tag
-- moves, and "roll back to yesterday's latest" means nothing if
-- yesterday's latest is today's. Deployments run the digest.
CREATE TABLE release (
    id         TEXT PRIMARY KEY,
    service_id TEXT    NOT NULL REFERENCES service (id) ON DELETE CASCADE,
    -- The reference as somebody would type it: repository and tag.
    reference  TEXT    NOT NULL,
    digest     TEXT    NOT NULL,
    -- 'push' when the registry received it, 'manual' when somebody
    -- pointed the service at an image by hand.
    source     TEXT    NOT NULL,
    created_at INTEGER NOT NULL,
    -- Set on the one currently deployed. Not derived from "the newest"
    -- because a rollback makes an older release the current one, which
    -- is the whole feature.
    deployed_at INTEGER
);

CREATE INDEX release_service ON release (service_id, created_at DESC);
-- The same digest can arrive twice under different tags; the same tag
-- and digest for one service is the same release.
CREATE UNIQUE INDEX release_unique ON release (service_id, reference, digest);

-- The environment, as it was.
--
-- Kept separately from releases on purpose. Rolling back an image and
-- rolling back a configuration are different intentions: the usual
-- case is "this build is bad, run the previous one, keep the settings
-- I fixed since". Tying them together would make one impossible.
CREATE TABLE config_revision (
    id         TEXT PRIMARY KEY,
    service_id TEXT    NOT NULL REFERENCES service (id) ON DELETE CASCADE,
    env        TEXT    NOT NULL,
    -- Who changed it, and what they were doing — 'edit' or 'revert'.
    changed_by TEXT    REFERENCES account (id) ON DELETE SET NULL,
    reason     TEXT    NOT NULL,
    created_at INTEGER NOT NULL
);

CREATE INDEX config_revision_service ON config_revision (service_id, created_at DESC);

-- Which tag a service watches, and whether a push to it deploys.
--
-- Automation on by default: a node where CI has to be told twice —
-- once to push, once to deploy — is one where the second half gets
-- forgotten and somebody debugs a version that never went out.
-- NULL means "whatever tag its image reference names". Derived in
-- code rather than filled in here: a reference can carry a registry
-- port (`host:5000/name:tag`), so finding the tag means finding the
-- *last* colon after the last slash — which SQLite has no clean way to
-- say, and which is worth a tested function rather than a clever
-- expression nobody will re-read.
ALTER TABLE service ADD COLUMN track_tag TEXT;
ALTER TABLE service ADD COLUMN auto_deploy INTEGER NOT NULL DEFAULT 1;
