-- Passkeys, trusted devices, approvals from them, one-time links and what
-- happened to an account (`docs/access.md`).
--
-- ## No password column change
--
-- `account.password_hash` stays NOT NULL, and an account without a
-- password holds the empty string, which no hash verifies against. SQLite
-- cannot drop a NOT NULL without rebuilding the table, and four others
-- reference this one.

-- One row per credential. The id is the credential's own, as base64url.
-- `rp_id` is the host it was made for, because a passkey answers for that
-- name only and a rename leaves it unusable.
CREATE TABLE passkey (
    id           TEXT PRIMARY KEY,
    account_id   TEXT    NOT NULL REFERENCES account (id) ON DELETE CASCADE,
    rp_id        TEXT    NOT NULL,
    public_key   BLOB    NOT NULL,
    sign_count   INTEGER NOT NULL DEFAULT 0,
    name         TEXT    NOT NULL,
    synced       INTEGER NOT NULL DEFAULT 0,
    created_at   INTEGER NOT NULL,
    last_used_at INTEGER
);

CREATE INDEX passkey_account ON passkey (account_id);

-- A ceremony between the page that began it and the POST that ends it.
-- Spent on use. `user_handle` is the id an account will be given, minted
-- before it exists, when the ceremony is the one that makes it.
CREATE TABLE passkey_challenge (
    challenge   TEXT PRIMARY KEY,
    purpose     TEXT    NOT NULL,
    account_id  TEXT,
    user_handle TEXT,
    expires_at  INTEGER NOT NULL
);

CREATE INDEX passkey_challenge_expires ON passkey_challenge (expires_at);

-- A browser an account trusts. The token is the browser's
-- `__Host-wabot_device` cookie, hashed; one browser may be trusted by
-- several accounts, so the pair is what is unique.
CREATE TABLE trusted_device (
    id           TEXT PRIMARY KEY,
    account_id   TEXT    NOT NULL REFERENCES account (id) ON DELETE CASCADE,
    token_hash   TEXT    NOT NULL,
    name         TEXT    NOT NULL,
    passkey_id   TEXT,
    created_at   INTEGER NOT NULL,
    last_used_at INTEGER NOT NULL,
    last_address TEXT
);

CREATE UNIQUE INDEX trusted_device_browser ON trusted_device (token_hash, account_id);

-- Which device a session was made on, and when its holder last proved who
-- they are. A session with no device ends with the browser.
ALTER TABLE session ADD COLUMN device_id TEXT;
ALTER TABLE session ADD COLUMN verified_at INTEGER;

-- The cookie's name changes to `__Host-wabot_session`, so no session from
-- before is presented again.
DELETE FROM session;

-- A new device asking to be let in, after its passkey passed. The browser
-- holds `__Host-wabot_pending`; a trusted device's "It was me" sets the
-- code it has to type.
CREATE TABLE device_request (
    id           TEXT PRIMARY KEY,
    account_id   TEXT    NOT NULL REFERENCES account (id) ON DELETE CASCADE,
    browser_hash TEXT    NOT NULL UNIQUE,
    passkey_id   TEXT    NOT NULL,
    name         TEXT    NOT NULL,
    address      TEXT,
    code_hash    TEXT,
    attempts     INTEGER NOT NULL DEFAULT 0,
    created_at   INTEGER NOT NULL,
    expires_at   INTEGER NOT NULL
);

CREATE INDEX device_request_account ON device_request (account_id);

-- A one-time way in. `issued_by_name` is kept in words so the event its
-- owner reads still says who after that person is gone; NULL for both is
-- root, on the terminal.
CREATE TABLE access_link (
    token_hash     TEXT PRIMARY KEY,
    account_id     TEXT    NOT NULL REFERENCES account (id) ON DELETE CASCADE,
    purpose        TEXT    NOT NULL,
    issued_by      TEXT,
    issued_by_name TEXT,
    created_at     INTEGER NOT NULL,
    expires_at     INTEGER NOT NULL,
    used_at        INTEGER
);

CREATE INDEX access_link_account ON access_link (account_id);

-- What happened to an account, for its owner to read. What they have read
-- is the last event's id rather than a time: two events in one
-- millisecond are still two.
CREATE TABLE account_event (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    account_id TEXT    NOT NULL REFERENCES account (id) ON DELETE CASCADE,
    kind       TEXT    NOT NULL,
    detail     TEXT    NOT NULL DEFAULT '',
    address    TEXT,
    at         INTEGER NOT NULL
);

CREATE INDEX account_event_account ON account_event (account_id, at);

ALTER TABLE account ADD COLUMN activity_seen_id INTEGER;
