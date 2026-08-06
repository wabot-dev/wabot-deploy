-- A network per project, an address per service.
--
-- Until now containers shared the host's network namespace, which made
-- a service's port a node-wide resource: two projects could not both
-- run something on 8080, and an image that binds a fixed port — most
-- of them — collided with the node's own edge. Each project now gets a
-- bridge and a /24, so the port inside the container is the port the
-- image chose.

-- The project's third octet in 10.42.0.0/16, and the number in its
-- bridge name. Allocated on first deploy rather than at creation: a
-- project that never runs anything should not be holding a subnet, and
-- there are only 254 of them.
--
-- NULL means "none yet". The partial unique index lets that repeat
-- while keeping the allocated ones distinct — which is what makes the
-- allocation safe under two deploys at once: the loser of the insert
-- retries rather than sharing a subnet.
ALTER TABLE project ADD COLUMN network_index INTEGER;

CREATE UNIQUE INDEX project_network_index
    ON project (network_index) WHERE network_index IS NOT NULL;

-- Where the proxy reaches this service: the address CNI gave the
-- container, paired with the service's own container_port.
--
-- Cleared when the service stops. An address left behind after the
-- container is gone is a route to nothing, and the reservation belongs
-- to whoever holds it next.
ALTER TABLE service ADD COLUMN address TEXT;
