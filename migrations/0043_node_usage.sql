-- What a node reports about how full it is.
--
-- A node's page read "no lo sabemos desde aquí" for every machine but
-- this one, because the report carried replicas, an endpoint and a list
-- of permissions and nothing about the machine itself. Four numbers make
-- the network page answer the question somebody opens it with.
--
-- Totals, not the breakdown. Which process holds which megabyte is that
-- machine's internals and its own console shows them; what another node
-- needs is "is it running out". Sending the parts would be more wire for
-- one card nobody can act on from here.
--
-- Nullable, and they stay null for a node that has not reported since
-- this column existed. A page that says nothing is right about a node it
-- has not heard from; a zero would read as an empty machine.
ALTER TABLE node ADD COLUMN memory_total INTEGER;
ALTER TABLE node ADD COLUMN memory_used INTEGER;
ALTER TABLE node ADD COLUMN disk_total INTEGER;
ALTER TABLE node ADD COLUMN disk_free INTEGER;
