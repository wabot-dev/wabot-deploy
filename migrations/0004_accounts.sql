-- Who may operate this node.

-- One row per operator. There is exactly one today — the admin created
-- at setup — but a table rather than a setting, because "add a second
-- person" should be a row and not a migration.
CREATE TABLE account (
    id            TEXT PRIMARY KEY,
    username      TEXT    NOT NULL,
    -- argon2id, in PHC string format. Never the password.
    password_hash TEXT    NOT NULL,
    created_at    INTEGER NOT NULL,
    last_seen_at  INTEGER
);

-- Case-insensitive, because an operator who registered `Admin` and
-- types `admin` at 3am should get in.
CREATE UNIQUE INDEX account_username ON account (lower(username));

-- Browser sessions.
--
-- A table rather than a signed token. The node has SQLite open
-- already, so the lookup is microseconds, and it buys the thing a JWT
-- cannot give without extra machinery: a session can be revoked. On a
-- box that deploys containers, "log this out now" is worth more than
-- saving a local read.
CREATE TABLE session (
    -- sha256 of the cookie value. Storing the value itself would mean
    -- a database anyone reads is a database anyone logs in with.
    token_hash TEXT PRIMARY KEY,
    account_id TEXT    NOT NULL REFERENCES account (id) ON DELETE CASCADE,
    created_at INTEGER NOT NULL,
    expires_at INTEGER NOT NULL
);

CREATE INDEX session_expires ON session (expires_at);
CREATE INDEX session_account ON session (account_id);

-- One-off values with a name. The setup token lives here.
CREATE TABLE setting (
    key        TEXT PRIMARY KEY,
    value      TEXT    NOT NULL,
    updated_at INTEGER NOT NULL
);
