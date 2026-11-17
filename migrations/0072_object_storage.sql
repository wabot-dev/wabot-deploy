-- A store of buckets is a service, and the difference is a row: the same
-- construction as `database`, for the same reason. See `docs/buckets.md`.
CREATE TABLE object_store (
    service_id  TEXT PRIMARY KEY REFERENCES service (id) ON DELETE CASCADE,

    -- Garage's series, '2'. The image's exact tag is the policy's and moves
    -- with a release of this node: Garage publishes no tag that follows a
    -- series.
    version     TEXT NOT NULL,

    -- 32 bytes as hex, which Garage's RPC refuses to start without even
    -- when it is the only node.
    rpc_secret  TEXT NOT NULL,
    -- Everything the node tells Garage goes through its admin API, CORS and
    -- expiry included, so this is the only credential the node holds.
    admin_token TEXT NOT NULL,

    created_at INTEGER NOT NULL
);

CREATE TABLE bucket (
    id          TEXT PRIMARY KEY,
    service_id  TEXT NOT NULL REFERENCES object_store (service_id) ON DELETE CASCADE,
    name        TEXT NOT NULL,
    -- Bytes, and Garage's quota for it. Garage has quotas per bucket and
    -- none per store, so the buckets' sum against the store's disk ceiling
    -- is the node's rule.
    max_size    INTEGER NOT NULL,
    public_read INTEGER NOT NULL DEFAULT 0,
    -- One origin per line, '' for none.
    cors_origins TEXT NOT NULL DEFAULT '',
    expire_days        INTEGER,
    -- An upload nobody completed holds disk and appears in no listing.
    abort_uploads_days INTEGER DEFAULT 7,
    created_at INTEGER NOT NULL,
    -- The store is the namespace: an endpoint per store is what bought it.
    UNIQUE (service_id, name)
);

CREATE TABLE bucket_key (
    id            TEXT PRIMARY KEY,
    bucket_id     TEXT NOT NULL REFERENCES bucket (id) ON DELETE CASCADE,
    name          TEXT NOT NULL,
    -- What Garage minted, kept so a store that lost it can be given it back.
    access_key_id TEXT NOT NULL UNIQUE,
    secret_key    TEXT NOT NULL,
    -- 'read_write' or 'read_only'.
    access        TEXT NOT NULL,
    created_at    INTEGER NOT NULL
);
