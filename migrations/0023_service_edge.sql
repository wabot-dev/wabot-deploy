-- Which public nodes answer for a service's hostname.
--
-- Chosen by the node that owns the service, like everything else about
-- it: where its replicas run, and who serves them. An edge can be any
-- public node on the network, including this one.
--
-- ## Why a table rather than a column on the port
--
-- A name can be served by several nodes at once — that is the point,
-- and it is what makes a name survive one of them going away. A column
-- would hold one.
--
-- The hostname is carried rather than derived from the port so that
-- this row still says what it was for after somebody changes the port's
-- name: the errand already sent named the old one, and the node serving
-- it has to be told to stop.
CREATE TABLE service_edge (
    service_id TEXT    NOT NULL REFERENCES service (id) ON DELETE CASCADE,
    hostname   TEXT    NOT NULL,
    -- The public node asked to answer for it. Not a foreign key:
    -- forgetting a node deletes its row, and an edge left pointing at
    -- nobody is worth keeping until somebody looks at the page.
    node_id    TEXT    NOT NULL,
    created_at INTEGER NOT NULL,
    PRIMARY KEY (hostname, node_id)
);

CREATE INDEX service_edge_service ON service_edge (service_id);
