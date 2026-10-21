-- A name that sends whoever asks for it to one of the service's own.
--
-- Asked for by Jorge on 2026-10-20: www.teresaasprilla.com should land on
-- teresaasprilla.com. A port carries one hostname, so the second name had
-- nowhere to go but a second service.
--
-- It points at a port rather than at a hostname: the target is whatever
-- that port answers to, so the redirect cannot outlive it or point at a
-- name the service no longer has — deleting the port, or the service,
-- takes it. Which nodes answer for the redirect's own name is a
-- `service_edge` row like any hostname's, which is what gets it a
-- certificate and tells the other nodes about it.
CREATE TABLE service_redirect (
    hostname   TEXT    PRIMARY KEY,
    port_id    TEXT    NOT NULL REFERENCES port (id) ON DELETE CASCADE,
    created_at INTEGER NOT NULL
);

CREATE INDEX service_redirect_port ON service_redirect (port_id);
