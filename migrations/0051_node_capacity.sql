-- What a node has left to promise, as it last reported.
--
-- ## Why the usage columns were not enough
--
-- `0043` added four numbers and they answer "is it running out" — which
-- is the question somebody opens the network page with, and not the one
-- a placement asks. Used memory is what containers are touching right
-- now; a node with 200 MB in use may have promised every byte it has to
-- services that happen to be idle. Placing against a reading is placing
-- against how busy the machine is this second.
--
-- So these are the other half: what the node will *accept*, and what it
-- has already undertaken. Allocatable is total minus the reserve that
-- machine's operator set, which is why it is computed there and sent as
-- an answer rather than as ingredients: a reader recomputing
-- `total - default` would contradict the operator of a machine it does
-- not administer.
--
-- ## And they are still never "something changed"
--
-- A commitment moves only when somebody places or resizes, so unlike a
-- reading it is a decision. It travels the same road anyway —
-- `note_reachability`, which returns `()` and structurally cannot feed
-- the caller's count — because `memory_allocatable` hangs off
-- `memory_total`, and one number flickering is enough to rebuild the
-- route table, rewrite every container's `/etc/hosts` and wake the
-- certificate loop every fifteen seconds. That storm is not worth
-- re-earning for a distinction nothing downstream would use.
--
-- ## Nullable, and all five or none
--
-- A node that has not reported since these existed has no answer, and
-- that is different from a node with nothing left. `fits` refuses to
-- enforce a rule made of a missing number, and a selector must not
-- *choose* a machine it knows nothing about — the two halves of the
-- same rule, and the reason `None` has to stay tellable from zero.
--
-- ## No disk here
--
-- Deliberately. A commitment needs something committed, and there is no
-- `service.disk_limit` to sum: the column arrives with the work that
-- decides what a disk reservation promises, since the volumes share a
-- filesystem and nothing enforces a share of it. A column that could
-- only ever hold zero would read as a node with no disk spoken for.
ALTER TABLE node ADD COLUMN memory_allocatable INTEGER;
ALTER TABLE node ADD COLUMN memory_committed INTEGER;
ALTER TABLE node ADD COLUMN cpu_allocatable INTEGER;
ALTER TABLE node ADD COLUMN cpu_committed INTEGER;

-- How many copies it holds with no ceiling at all.
--
-- Not folded into the sums, because it cannot be: a container with no
-- limit may take the machine, and adding a guess would make the
-- arithmetic look complete while being wrong by an unknown amount.
-- Carried so a reader can say the sum is a floor rather than a total —
-- which is the honest thing to put on a page, and what stops somebody
-- placing against a number that quietly under-reports.
ALTER TABLE node ADD COLUMN unbounded INTEGER;
