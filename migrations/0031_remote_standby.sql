-- A standby on another node, and telling that node how to reach the
-- primary.
--
-- ## Why the endpoint is stored rather than derived
--
-- The node that *owns* a database works out where its primary answers
-- by reading its own rows: the replica in `primary_slot`, its reserved
-- address, its overlay port. A node that was *asked* to run a standby
-- has none of those rows — it has never heard of the primary, and the
-- copy it holds is the only one it knows about.
--
-- So the errand carries the endpoint and this is where it lands. NULL
-- means "derive it", which is what the owning node does; a value means
-- "you were told", which is what a receiving node reads. The two
-- answers are the same address and they are arrived at differently,
-- which is exactly the shape `replica.node_id` already uses for "here".
ALTER TABLE database ADD COLUMN primary_endpoint TEXT;

-- What an errand was about.
--
-- ## Because a database's errands are recomputed, not emitted
--
-- Every other errand is queued by somebody pressing a button, once. A
-- database's cannot be: the port its primary answers on comes out of
-- the *other* node's port space and travels home on a report, so the
-- instruction has to be rebuilt whenever the facts change and queued
-- only if it differs from the last one.
--
-- Comparing against "the last one" needs a way to find it, and "the
-- most recent errand for this node of this kind" is not enough — one
-- node can hold standbys of two databases. This is the subject: which
-- thing the instruction is about.
--
-- The errands page wanted it anyway. "An edge errand to that node" is
-- not an answer when the same node serves two of a service's names.
ALTER TABLE errand ADD COLUMN subject TEXT;

CREATE INDEX errand_subject ON errand ("node_id", "subject");
