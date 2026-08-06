-- The edge: what certificate to present, and where a hostname goes.

-- Exactly one local certificate authority, generated on first start.
--
-- A CA rather than a bare self-signed leaf so the operator trusts one
-- thing once: the leaf is reissued whenever the node's names change —
-- a domain is configured, an address moves — and re-trusting on every
-- change is how people learn to click through warnings.
--
-- The CHECK is the schema saying "one row" out loud, rather than the
-- code remembering to.
CREATE TABLE local_ca (
    id         INTEGER PRIMARY KEY CHECK (id = 1),
    cert_pem   TEXT    NOT NULL,
    key_pem    TEXT    NOT NULL,
    created_at INTEGER NOT NULL
);

-- One row per hostname the node can present a certificate for.
--
-- `not_after` is stored rather than parsed out of the PEM on demand:
-- the renewal loop asks "what expires soon" on a schedule, and that is
-- a query, not a scan-and-decode.
CREATE TABLE certificate (
    domain     TEXT PRIMARY KEY,
    -- Every name the certificate covers, sorted, comma-separated.
    --
    -- Stored rather than decoded back out of the DER: deciding whether
    -- to reissue is "does this cover what I need now", which is a set
    -- comparison against what we asked for. Recovering the SANs from
    -- the certificate would mean a decoder to maintain, and would
    -- still not see an IP SAN as the text that was requested.
    names      TEXT    NOT NULL,
    cert_pem   TEXT    NOT NULL,
    key_pem    TEXT    NOT NULL,
    -- 'self-signed' now; 'acme' once M2 lands. Kept as text rather
    -- than a boolean because a third issuer is likelier than not.
    issuer     TEXT    NOT NULL,
    issued_at  INTEGER NOT NULL,
    not_after  INTEGER NOT NULL,
    last_error TEXT
);

CREATE INDEX certificate_not_after ON certificate (not_after);

-- Hostname -> where its traffic goes.
--
-- The control plane is a row like any other, so "which host serves the
-- console" is data an operator can read and change, not a constant
-- compiled into the dispatch.
CREATE TABLE route (
    host          TEXT PRIMARY KEY,
    -- 'control_plane' | 'proxy'
    upstream_kind TEXT    NOT NULL,
    -- host:port, for 'proxy'. NULL otherwise.
    upstream_addr TEXT,
    -- Which service owns this route, once services exist.
    service_id    TEXT,
    enabled       INTEGER NOT NULL DEFAULT 1,
    updated_at    INTEGER NOT NULL
);
