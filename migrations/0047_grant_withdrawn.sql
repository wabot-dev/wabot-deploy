-- What the operator has refused a node, whatever its join agreed.
--
-- `node_grant` is the agreement; this is somebody changing their mind
-- afterwards. It has to be a table rather than the absence of a grant
-- row, and the reason is written on `network::honour_offers`: that pass
-- runs at every boot and re-adds whatever the enrolment offered, because
-- it asks about the enrolment and not about the history. A withdrawal
-- expressed by deleting a row would be undone by the next restart, and
-- the operator would have no way to tell that from it never having
-- worked.
--
-- Read by `capability::granted_to`, which filters what it finds through
-- this — so a path that grants blindly is bounded by the more recent
-- decision rather than trusted to remember it.
CREATE TABLE node_grant_withdrawn (
    -- Not a foreign key to `node`, for the reason `node_grant` is not:
    -- an authority is recorded before anything else is known about it.
    node_id      TEXT    NOT NULL,
    capability   TEXT    NOT NULL,
    withdrawn_at INTEGER NOT NULL,
    PRIMARY KEY (node_id, capability)
);
