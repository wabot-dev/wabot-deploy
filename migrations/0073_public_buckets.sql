-- A public bucket is read at a name of its own, routed to its store's
-- website port, where Garage finds the bucket by that name. The column says
-- which of the store's names is this bucket's; null is private.
ALTER TABLE bucket ADD COLUMN public_hostname TEXT;
ALTER TABLE bucket DROP COLUMN public_read;

-- So one container port answers several names: every public bucket of a
-- store is a name on port 3902. A port with no name is still declared once,
-- and a name is still one service's.
DROP INDEX port_service_container;
CREATE UNIQUE INDEX port_service_container ON port (service_id, container_port)
    WHERE hostname IS NULL;
