-- The port a node's WireGuard listens on, as that node said it.
--
-- A node behind NAT whose tunnel to its authority has died moves to a new
-- port, because a router can hold stale state on one pair of addresses and
-- ports and let every other pair through (`docs/network.md`, phase 13). So
-- the port stops being something each node assumes is its own and becomes
-- part of where a node is: a LAN sibling dials it there, and an
-- `overlay.port` that differs between two nodes no longer keeps them apart.
--
-- **Outside `COLUMNS`, `read` and `save`** in `network::mod`, for the
-- reason `lan` is (migration 0046): `save` writes the whole row and
-- `ensure_self` calls it. Written by the report and the census that carried
-- it, and on this node's own row by the move, which is the only writer that
-- knows. NULL on another node's row is one that has never said, which is an
-- older one, dialled at this node's configured port as before. NULL on this
-- node's own row is one that has never moved, listening on its configured
-- port.
ALTER TABLE node ADD COLUMN "listen_port" INTEGER;

-- What this node last did about a tunnel that had stopped working, per
-- peer: run `ensure` again, move to a new port, or forget an address it had
-- learned. A repair nobody can see is one nobody can trust or debug, and
-- the loop that does it runs in `serve` while `doctor` is another process,
-- so it is written down rather than held.
CREATE TABLE tunnel_repair (
    "node_id" TEXT PRIMARY KEY,
    "at" INTEGER NOT NULL,
    "did" TEXT NOT NULL
);
