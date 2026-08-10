-- Being an edge is a choice, including for the node that owns the
-- service.
--
-- It used to be automatic: this node built a route for every hostname on
-- its own ports, and `service_edge` only ever held *other* nodes. That
-- read the model backwards. The only thing that separates a private node
-- from a public one is whether it exposes its own address — so a private
-- node can own projects and services perfectly well, and have them served
-- from somewhere else entirely. If the owner is not necessarily the edge,
-- then the owner serving its own names is a decision like any other and
-- belongs in the same table.
--
-- Every hostname that exists now keeps being served by this node, which
-- is what it is doing today and what the operator expects to keep
-- happening. Only if this node *can* be an edge: a private one never was
-- reachable for those names, and writing the row would tell the console
-- it is doing something it cannot do.
INSERT OR IGNORE INTO service_edge ("service_id", "hostname", "node_id", "created_at")
SELECT "port"."service_id", "port"."hostname", "node"."id", CAST(strftime('%s','now') AS INTEGER) * 1000
FROM "port"
JOIN "node" ON "node"."is_self" = 1
WHERE "port"."hostname" IS NOT NULL
  AND "node"."endpoint" IS NOT NULL
  AND "node"."kind" = 'public';
