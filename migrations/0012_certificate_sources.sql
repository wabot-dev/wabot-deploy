-- Where a certificate comes from, and how it is kept.
--
-- `issuer` was doing two jobs. It recorded who signed a certificate — a
-- fact, shown by `doctor` and the console — and it also decided whether
-- to reissue, by comparing itself against the configured ACME directory
-- (`acme::ensure`). That works while there are two sources and the
-- choice is global. It stops working the moment a certificate can come
-- from somewhere the node did not ask: the comparison fails, and the
-- renewal loop replaces the operator's own certificate without a word.
--
-- So the two jobs split. `source` says where what is installed came
-- from; `certificate_policy` says what to do about it next.

-- 'self_signed' | 'acme' | 'file'
ALTER TABLE certificate ADD COLUMN source TEXT NOT NULL DEFAULT 'self_signed';

-- Anything this node did not sign itself came from an authority. There
-- is no third case yet — this migration is what creates one.
UPDATE certificate SET source = 'acme' WHERE issuer <> 'self-signed';

-- How a name's certificate is kept.
--
-- A table of its own rather than columns beside the name, because a
-- policy precedes and outlives any certificate it produces: a name is
-- configured before anything has been issued for it, and the choice has
-- to survive the certificate being replaced.
--
-- No row means the default, which is what every name has until somebody
-- says otherwise — so configuring nothing is a state the schema can
-- represent rather than a row somebody has to remember to write. A row
-- naming something that is no longer served is inert: the renewal loop
-- reads policies for the names it wants, never the other way round.
CREATE TABLE certificate_policy (
    name       TEXT PRIMARY KEY,
    -- 'acme' | 'self_signed' | 'file'
    renew_with TEXT    NOT NULL,
    -- For 'file': where to read the certificate and its key from.
    --
    -- The node does not renew these — it cannot, it has no relationship
    -- with whoever signed them. Something else keeps the files fresh
    -- (cert-manager, certbot, a corporate tool) and the node reinstalls
    -- what it finds. That is the only reading of "renew an uploaded
    -- certificate" the node can actually honour.
    cert_path  TEXT,
    key_path   TEXT,
    updated_at INTEGER NOT NULL
);
