-- A username belongs to a frontier, not to the machine.
--
-- ## The clash this ends
--
-- `account_username` was unique across the whole node, so two
-- organisations could not both have somebody called `admin`, and an
-- organisation's first person could be refused a name because the
-- machine's operator had taken it — a refusal that names somebody they
-- cannot see and have no way to negotiate with.
--
-- Under tenancy that is the wrong shape entirely. `jorge` at Acme,
-- `jorge` at Globex and `jorge` who runs the machine are three people,
-- and nothing about one is any of the others' business.
--
-- ## COALESCE, and why the obvious index is wrong
--
-- `UNIQUE (lower(username), tenant_id)` looks right and is not: SQLite
-- follows the SQL standard and treats NULLs as **distinct** in a unique
-- index, so two operators of the machine — both `tenant_id IS NULL` —
-- would collide with nothing at all. The global uniqueness the node has
-- always had would be silently gone, which is worse than the clash this
-- migration is fixing.
--
-- `COALESCE("tenant_id", '')` maps every NULL to one value, so the
-- machine's own scope is a scope like any other. An organisation id is
-- never empty, so there is nothing for it to collide with.
--
-- ## And the lookup has to follow
--
-- Signing in now asks for a username *within a frontier*, and the
-- frontier is the `Host` the request arrived at. A sign-in that still
-- searched the whole table would find whichever of the three rows came
-- first, which is somebody else's account.
DROP INDEX IF EXISTS account_username;

CREATE UNIQUE INDEX account_username
    ON account (lower("username"), COALESCE("tenant_id", ''));
