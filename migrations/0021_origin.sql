-- Where a project or a service came from, when it did not come from
-- here.
--
-- ## Why this is not a flag
--
-- "Foreign" would be enough to refuse an edit, and it would not be
-- enough to say anything useful in the danger zone that allows the one
-- thing an operator can always do. Somebody looking at a service they
-- did not create needs to know *which node* to go and argue with, and
-- a boolean cannot tell them.
--
-- Null is this node's own, the same way `replica.node_id` is null for
-- here and `node.endpoint` is null for a node the world cannot dial.
-- The absence is the answer, rather than a second value meaning "me"
-- that every writer has to remember to fill in correctly.
--
-- ## What it is for
--
-- A service is administered from the node that created it. What lands
-- somewhere else is derived: not editable there, because two nodes
-- disagreeing about one service has no way to settle. Evictable there,
-- because the machine belongs to whoever runs it even when the orders
-- do not — the same rule the grant already follows in the other
-- direction.
ALTER TABLE project ADD COLUMN origin_node_id TEXT;
ALTER TABLE service ADD COLUMN origin_node_id TEXT;

-- Everything that exists now was created here. A node upgrading into
-- this has nothing foreign on it: the only way to get a foreign row is
-- an errand, and errands are newer than every row this runs against.
