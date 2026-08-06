-- The ACME account, and the challenges in flight.

-- One row per directory URL, so staging and production accounts can
-- coexist: switching between them is a config change, and losing the
-- production account because somebody tested against staging would be
-- a bad afternoon.
--
-- `credentials` is instant-acme's own serialized form, which carries
-- the account key. It is the reason the data directory is 0700.
CREATE TABLE acme_account (
    directory_url TEXT PRIMARY KEY,
    email         TEXT,
    credentials   TEXT    NOT NULL,
    created_at    INTEGER NOT NULL
);

-- HTTP-01 challenges awaiting validation.
--
-- In the database rather than in memory because the answer has to
-- survive a restart: an order can be mid-flight when the node is
-- upgraded, and a challenge the CA asks about after that would
-- otherwise 404 and fail the order.
--
-- `expires_at` is what stops this growing forever when an order is
-- abandoned.
CREATE TABLE acme_challenge (
    token      TEXT PRIMARY KEY,
    -- What to answer with: the key authorization.
    response   TEXT    NOT NULL,
    domain     TEXT    NOT NULL,
    expires_at INTEGER NOT NULL
);

CREATE INDEX acme_challenge_expires ON acme_challenge (expires_at);
