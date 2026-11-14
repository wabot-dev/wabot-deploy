-- What a node holding the primary for somebody else is told, and cannot
-- work out from its own rows. NULL (or 0) on a database this node owns,
-- which derives all three — the shape `primary_endpoint` and
-- `owner_domain` already have, and for the same reason.
--
-- Who may follow the primary: a JSON array of CIDRs, the overlay address
-- of every node holding a standby. A node holding the primary has rows for
-- its own slots and no others, so a `pg_hba.conf` built from what it knows
-- admits none of them, which is a standby that never follows.
ALTER TABLE database ADD COLUMN replication_from TEXT;

-- The name it answers to, chosen or derived, as the owner computes it. A
-- name somebody chose lives on the owner's port row, which never travels,
-- so without this the copy answered to the derived name while the owner's
-- page said otherwise.
ALTER TABLE database ADD COLUMN owner_name TEXT;

-- Whether the owner publishes it on a port. The port is opened on the
-- owner, which the name resolves to, and forwarded over the overlay; what
-- the holder needs to know is that clients may arrive from outside the
-- project, so `pg_hba.conf` admits them.
ALTER TABLE database ADD COLUMN owner_published INTEGER NOT NULL DEFAULT 0;
