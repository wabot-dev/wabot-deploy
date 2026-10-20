-- The name servers a zone sent here was sent with.
--
-- A zone administered here lists its servers by node in `dns_zone_server`
-- and names them at the moment it is served. One sent from elsewhere may
-- name machines this node has never heard of, so it keeps the names it was
-- given: comma-separated, in order, NULL on a zone of this node's own.
ALTER TABLE dns_zone ADD COLUMN name_servers TEXT;
