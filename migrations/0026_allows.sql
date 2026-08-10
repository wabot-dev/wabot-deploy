-- What another node lets *this* one ask of it.
--
-- `node_grant` is the decision, and it lives on the machine the decision
-- binds — which means the node doing the asking cannot read it. It is on
-- the other side of the network, in a database it has no access to and
-- should not have.
--
-- So the answer has to travel, and it lands here: a learned fact about a
-- remote node, beside `endpoint`, which is the other one. Both follow
-- the same rule the two-node run produced — **a row about another node
-- describes the relationship, not the machine** — and both arrive the
-- same way, on the report that node already sends, so an already-joined
-- node heals itself without re-joining.
--
-- ## Why the backfill is asymmetric, and how it can be
--
-- Today's relationships are whole in one direction and empty in the
-- other: A enrolled B, so B granted A everything and A granted B
-- nothing. Each node can work out which side it is on without asking
-- anybody — a node in this node's `authority` table is one it takes
-- orders *from*, and a node that gives orders was never granted
-- anything back.
--
-- Filling both directions with "everything" would have been the tidier
-- migration and would have re-created the exact bug this phase exists to
-- fix: a picker offering a node that will never collect the errand.
ALTER TABLE node ADD COLUMN allows TEXT;

UPDATE node
SET "allows" = CASE
    WHEN "id" IN (SELECT "node_id" FROM "authority") THEN ''
    ELSE 'host,edge'
END
WHERE "is_self" = 0;
