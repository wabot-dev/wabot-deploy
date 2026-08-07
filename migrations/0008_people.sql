-- People, and what each of them may do.
--
-- Two levels, because the node has two kinds of question. "May this
-- person create projects, invite people, look at the node's memory" is
-- about the node. "May this person deploy *here*" is about one
-- project, and the answer differs per project — which is the whole
-- reason projects exist.
--
-- An administrator is not a member of anything and can do everything:
-- membership answers the project question, and an admin never gets
-- that far. That is deliberate. A node where the administrator has to
-- add themselves to a project before they can fix it is a node that
-- locks its operator out of the thing they operate.

-- 'admin' or 'member'. Anything unrecognised reads as 'member' — the
-- code that parses this treats an unknown role as the least it could
-- mean, so a row written by a newer version cannot grant more than it
-- should to an older one.
ALTER TABLE account ADD COLUMN role TEXT NOT NULL DEFAULT 'member';

-- Whoever exists already came through setup, and setup creates the
-- administrator. There is at most one row here when this runs.
UPDATE account SET "role" = 'admin';

-- Who is in a project, and as what.
--
-- 'owner' — everything in it, including deleting it and managing who
-- else is in it. 'deployer' — services and ports. 'viewer' — read.
CREATE TABLE membership (
    account_id TEXT    NOT NULL REFERENCES account (id) ON DELETE CASCADE,
    project_id TEXT    NOT NULL REFERENCES project (id) ON DELETE CASCADE,
    role       TEXT    NOT NULL,
    created_at INTEGER NOT NULL,
    PRIMARY KEY (account_id, project_id)
);

CREATE INDEX membership_project ON membership (project_id);

-- An invitation is a one-shot token, stored hashed, exactly like the
-- setup token — and for the same reason: a database somebody reads
-- must not be a database somebody joins with.
--
-- The invitee chooses their own username and password. The person
-- inviting never sees either, which is the property that makes this
-- better than an administrator typing a password and sending it.
CREATE TABLE invitation (
    id           TEXT PRIMARY KEY,
    token_hash   TEXT    NOT NULL,
    -- What they become on the node.
    node_role    TEXT    NOT NULL,
    -- And, optionally, the project they land in and as what. An
    -- invitation with no project is somebody joining the node itself.
    project_id   TEXT    REFERENCES project (id) ON DELETE CASCADE,
    project_role TEXT,
    -- Who to blame, and when it stops working.
    created_by   TEXT    NOT NULL REFERENCES account (id) ON DELETE CASCADE,
    created_at   INTEGER NOT NULL,
    expires_at   INTEGER NOT NULL,
    -- Set when spent. Kept rather than deleted so the people page can
    -- say what happened to an invitation somebody is asking about.
    used_at      INTEGER,
    used_by      TEXT    REFERENCES account (id) ON DELETE SET NULL
);

CREATE UNIQUE INDEX invitation_token ON invitation (token_hash);
CREATE INDEX invitation_project ON invitation (project_id);
