-- A network of nodes, and who is allowed to configure whom.
--
-- ## Why there is no agreement to reach
--
-- The obvious shape for "several nodes share a configuration" is a
-- consensus protocol, and it is an order of magnitude more machinery
-- than this product should carry. It is also not what is wanted: a node
-- does not need to agree with its peers about the world, it needs to
-- know **which of them it takes instructions from**.
--
-- So every relationship here is directed. A node grants authority to
-- another; the other sends errands. Two nodes that never granted each
-- other anything cannot affect each other at all, and nothing has to be
-- reconciled because nothing is shared.

-- Every node this one knows about, including itself.
--
-- `node::all()` returned a synthetic list of one, and the module docs
-- said why the plural was there from the beginning: "a list that starts
-- as a detail page never becomes a list without breaking every link
-- into it." This is that list becoming real.
CREATE TABLE node (
    id           TEXT PRIMARY KEY,
    name         TEXT    NOT NULL,
    -- 'public'  — reachable from the internet, so it can terminate TLS
    --             for names whose containers live somewhere else.
    -- 'private' — runs containers, reached only across the overlay.
    --
    -- The difference is not a setting, it is whether `endpoint` is
    -- there: a node with no address the world can dial is private
    -- whatever it calls itself.
    kind         TEXT    NOT NULL,
    -- host:port a public node answers on.
    endpoint     TEXT,
    -- Filled in when the node joins the overlay. Here rather than in a
    -- table of their own because a node without them has not finished
    -- joining — it is not a different kind of thing.
    public_key   TEXT,
    overlay_ip   TEXT,
    is_self      INTEGER NOT NULL DEFAULT 0,
    joined_at    INTEGER NOT NULL,
    -- When this node last heard from it. A join that went quiet is the
    -- failure an operator most needs to see, and it has no error to
    -- report — only a silence.
    last_seen_at INTEGER
);

-- One node is this one. Enforced rather than assumed: two rows claiming
-- to be self would make every "am I the one that should act" question
-- answer twice.
CREATE UNIQUE INDEX node_is_self ON node (is_self) WHERE is_self = 1;
-- An overlay address belongs to one node, or routing sends traffic to
-- whichever row was read first.
CREATE UNIQUE INDEX node_overlay_ip ON node (overlay_ip) WHERE overlay_ip IS NOT NULL;

-- Who may configure this node.
--
-- The whole model in one table. A node decides which others it takes
-- errands from, and a node that was never granted anything can ask for
-- nothing. Joining is therefore not a loss of control: it is one row,
-- written deliberately, and revocable.
CREATE TABLE authority (
    -- The node being trusted. Not a foreign key: authority can be
    -- granted by a token before the granting node has been described,
    -- and a constraint here would make the order of two writes matter.
    node_id    TEXT PRIMARY KEY,
    -- What an errand from it has to carry, hashed. Like every other
    -- secret here, storage read access alone does not authenticate.
    token_hash TEXT    NOT NULL,
    granted_at INTEGER NOT NULL,
    -- Set rather than deleted. "This used to be allowed" is worth being
    -- able to read, and a revoked grant that vanished would look like
    -- one that never happened.
    revoked_at INTEGER
);

-- A name this node serves, and who asked it to.
--
-- One authority per name, and a second claim is **refused** rather than
-- merged or overwritten. Two nodes pointing one hostname at different
-- backends is not a conflict a machine can resolve, and choosing
-- silently would make the wrong answer look like the right one. The
-- refusal is reported, which is the only outcome somebody can act on.
CREATE TABLE claim (
    name         TEXT PRIMARY KEY,
    -- The node that asked. NULL when this node claimed the name for
    -- itself, which is what every name is today.
    authority_id TEXT,
    claimed_at   INTEGER NOT NULL
);
