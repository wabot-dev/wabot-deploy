-- An organisation whose projects live on this node.
--
-- ## What a tenant is, and what it deliberately is not
--
-- It is a **frontier of ownership**: a set of projects, the people who
-- may reach them, a domain their names hang off, and a ceiling on what
-- they may take. Everything under it is administered from this node, the
-- same way a service is administered from the node where it was created.
--
-- It is **not** a second control plane. There is no replicated database,
-- no lease, no quorum — `docs/network.md` says authority is directed and
-- there is no agreement to reach, and this does not change that. A
-- tenant's copies may run anywhere on the network that consented to run
-- them; its rows live here.
--
-- What that costs is stated rather than hidden: while this node is down,
-- a tenant cannot be *changed*. Its containers go on serving, because
-- the overlay, the port mappings and containerd all outlive this
-- process — and its administration comes back with `restore-node`, which
-- returns the identity whole and is verified on a real machine.
--
-- ## Why the columns are here and not on `project`
--
-- A quota is per organisation, not per project: a customer with five
-- projects has one budget, and five columns that had to be kept in
-- agreement is five chances to disagree. Same for the domain — a name is
-- the tenant's, and its projects are underneath it.
CREATE TABLE tenant (
    id         TEXT PRIMARY KEY,
    name       TEXT NOT NULL,
    -- What its console answers to, and the suffix its service names hang
    -- off. Unique because a `Host` reaches exactly one of these, and two
    -- rows claiming one name is not something a request can resolve.
    slug       TEXT NOT NULL,
    -- NULL until somebody sets one. A tenant with no domain still works:
    -- its names fall back to this node's, which is what every service
    -- did before tenants existed.
    domain     TEXT,
    -- The ceiling, in bytes and millicores and bytes. **NULL is no
    -- quota**, and it is what every tenant gets until somebody decides
    -- otherwise — the same rule `service.memory_limit` follows, and for
    -- the same reason: a default here would be this migration deciding
    -- that somebody's organisation now gets refused at a number nobody
    -- chose.
    --
    -- The limit is the reservation, as everywhere else in this codebase.
    -- See `migrations/0038_cpu_limit.sql` for the argument against a
    -- second number for a request.
    quota_memory   INTEGER,
    quota_cpu      INTEGER,
    quota_disk     INTEGER,
    created_at INTEGER NOT NULL
);

CREATE UNIQUE INDEX tenant_slug ON tenant (lower(slug));

-- Which tenant a project belongs to. NULL is the node's own.
--
-- The same spelling `project.origin_node_id` and `replica.node_id`
-- already use, and for the same reason: the absence is the answer, so
-- every row that existed before this migration means what it always
-- meant without being rewritten.
--
-- **Not a cascade.** Deleting a tenant with projects in it would take
-- somebody's containers and volumes with it in one statement — which is
-- the kind of thing this codebase refuses at the form and says why, the
-- way `capability::owns` refuses to be turned off while the node still
-- owns projects.
ALTER TABLE project ADD COLUMN tenant_id TEXT REFERENCES tenant (id);

-- And which tenant a person belongs to. NULL is the node's own operator.
--
-- ## Why this is not a role
--
-- `NodeRole` answers "how much may they do", and this answers "over
-- what". A third role would be a third thing every one of the checks
-- that ask `is_admin()` has to remember, and there are forty-three of
-- them. As a scope it is one clause inside `is_admin` itself, which is
-- the difference between a rule and a convention.
--
-- `role` keeps its meaning and gains a subject: administrator **of my
-- own scope**. So a tenant's administrator reaches every project of that
-- tenant without being a member of any — which is the same thing the
-- node's administrator has always been able to do, one frontier in.
ALTER TABLE account ADD COLUMN tenant_id TEXT REFERENCES tenant (id);

CREATE INDEX project_tenant ON project (tenant_id);
CREATE INDEX account_tenant ON account (tenant_id);
