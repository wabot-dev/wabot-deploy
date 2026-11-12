-- A TXT record the node wrote itself to prove it controls a name to a
-- certificate authority (DNS-01), as opposed to one somebody typed.
--
-- It is an ordinary record in every other way, so that the one place that
-- answers DNS and the one place that sends a zone to the other nodes need
-- not know about it. The marker is what lets the node remove exactly what
-- it wrote — after an order, and at boot after one that was interrupted —
-- without ever touching a `_acme-challenge` record a person added for
-- some other purpose.
ALTER TABLE dns_record ADD COLUMN acme_challenge INTEGER NOT NULL DEFAULT 0;
