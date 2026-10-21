-- A zone belongs to a project, like a service.
--
-- It belonged to an owner: the node's operator (`tenant_id` NULL) or an
-- organisation. Jorge asked on 2026-10-20 for zones to be a project's
-- resource, so the people of a project manage its domains the way they
-- manage its services, and deleting the project takes them with it.
--
-- Each zone administered here goes into the oldest project of the owner it
-- had — the node's own, or the organisation's — which is the only answer a
-- migration can give without asking anybody. One whose owner has no project
-- keeps `project_id` NULL: still answered, listed on the node's DNS tab, and
-- deletable there by the operator, and nothing else.
--
-- A zone received from another node (`authority_id`) belongs to no project
-- here; its project, if any, is the sender's business.
--
-- `tenant_id` goes: whose a zone is now follows from its project, and a
-- second column saying the same thing is one that can disagree with it.
ALTER TABLE dns_zone ADD COLUMN project_id TEXT REFERENCES project (id) ON DELETE CASCADE;

UPDATE dns_zone
SET project_id = (
    SELECT p."id" FROM project p
    WHERE p."tenant_id" IS dns_zone."tenant_id"
      AND p."origin_node_id" IS NULL
    ORDER BY p."created_at" ASC, p."id" ASC
    LIMIT 1
)
WHERE authority_id IS NULL;

ALTER TABLE dns_zone DROP COLUMN tenant_id;

CREATE INDEX dns_zone_project ON dns_zone (project_id);
