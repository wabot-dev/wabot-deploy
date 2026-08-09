-- What this node presents to a registry that will not serve strangers.
--
-- ## Keyed by host, not by service
--
-- A credential is a property of the registry, not of the thing being
-- pulled: every service whose image lives on the same node authenticates
-- the same way, and storing it per service would be the same secret
-- copied once per deployment with no way to rotate it in one place.
--
-- It is the same shape `~/.docker/config.json` has, for the same reason,
-- and it generalises past the case that brought it: an errand from an
-- authority writes one so the image can be pulled from *that* node's
-- registry, and nothing about the table knows that is why.
CREATE TABLE registry_credential (
    -- `host` or `host:port`, exactly as it appears in an image
    -- reference — that is what the pull path has to match against, and
    -- normalising here would mean matching a normalised form there.
    host       TEXT PRIMARY KEY,
    -- The OCI distribution spec says username and password. This node's
    -- own registry reads a push token as the password against any
    -- username, which is what the other end will be.
    username   TEXT    NOT NULL,
    -- In clear, and it has to be: this is a credential to *present*,
    -- not one to check. A hash could never be sent. Same reason the
    -- authority secret in `0018` is stored this way, and the same
    -- protection — a data directory created `0700`.
    secret     TEXT    NOT NULL,
    created_at INTEGER NOT NULL
);
