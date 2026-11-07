-- A name of a project's own that its images are pushed to and pulled from.
--
-- Asked for by Jorge on 2026-11-07. Docker keeps one login per registry
-- host, so every project behind the node's one name shared a single slot
-- in ~/.docker/config.json: logging in with one project's token logged
-- out of every other, and pushing to two projects meant logging in again
-- between them. A far node had the same slot and the same collision.
--
-- Null is the node's own name, which is what every project had before.
-- Unique among the projects that have one, because a host is routed to
-- one project's registry and two rows claiming it is a request nobody
-- can resolve.
ALTER TABLE project ADD COLUMN registry_domain TEXT;

CREATE UNIQUE INDEX project_registry_domain ON project (registry_domain)
    WHERE registry_domain IS NOT NULL;
