-- Who answers for a name that no port of the service has any more.
--
-- Deleting a port left its `service_edge` rows behind, and those rows are
-- what the renewal loop orders certificates for — so a name removed from
-- a service went on being asked of an authority for as long as the node
-- ran. Found on a VM node. `ports::delete` takes them with the port now;
-- this takes the ones it left before.
DELETE FROM service_edge
WHERE NOT EXISTS (
    SELECT 1 FROM port
    WHERE port."service_id" = service_edge."service_id"
      AND port."hostname" = service_edge."hostname"
);
