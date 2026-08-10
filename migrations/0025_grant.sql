-- What one node lets another ask of it, one capability at a time.
--
-- The `authority` table already says *who* may send errands here and
-- carries the secret they authenticate with. It does not say what for,
-- and until there was one kind of errand that was the same question.
-- With two it stopped being: a node may want another to serve its names
-- and never run its containers, and there was no way to say so.
--
-- ## Why the grant lives on the node being asked
--
-- Because that is the node the answer binds. A row here means "this
-- machine has agreed to do that for that node", written on the machine
-- that agreed, and revocable from it without asking anyone. The same
-- reason the authority row lives here: joining is not a loss of control.
--
-- ## Why every existing authority gets both
--
-- They work today. A migration that quietly narrowed a live grant would
-- take a running network off the air to make a table tidier, and the
-- operator would find out from a replica that stopped being placed
-- rather than from anything that said so.
CREATE TABLE node_grant (
    -- The node allowed to ask. Not a foreign key to `node`: an
    -- authority is recorded before anything else is known about it, and
    -- a grant that could not be written until the row existed would
    -- order the join backwards.
    node_id    TEXT    NOT NULL,
    -- `host` or `edge`. Stored as text rather than a number so a
    -- database somebody opens by hand reads as what it is, and so an
    -- unknown one from a newer node is legible rather than a mystery
    -- integer.
    capability TEXT    NOT NULL,
    granted_at INTEGER NOT NULL,
    PRIMARY KEY (node_id, capability)
);

INSERT OR IGNORE INTO node_grant ("node_id", "capability", "granted_at")
SELECT "node_id", 'host', CAST(strftime('%s','now') AS INTEGER) * 1000 FROM "authority";

INSERT OR IGNORE INTO node_grant ("node_id", "capability", "granted_at")
SELECT "node_id", 'edge', CAST(strftime('%s','now') AS INTEGER) * 1000 FROM "authority";

-- What a token asks for and what it hands over, so the node spending it
-- can read the terms before it commits. Comma-separated capability
-- names: two of them, in one column that is only ever read whole.
--
-- Empty means "nothing", not "everything". A token minted before this
-- existed has no row to be empty, and `enrolment::all` reads NULL as
-- both — the terms those tokens were minted under.
ALTER TABLE enrolment ADD COLUMN requires TEXT;
ALTER TABLE enrolment ADD COLUMN offers TEXT;
