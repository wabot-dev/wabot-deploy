-- Enrolment: how a node comes to take errands from another.
--
-- The public node mints a token and shows it once; somebody carries it
-- to the private node and runs `wabot-deploy join <token>`. That node
-- writes the row that grants authority, and then calls back here to say
-- who it is. Two writes, in opposite directions, and neither of them is
-- shared state — see migration `0015`.
--
-- ## Why the address is allocated now rather than on arrival
--
-- The overlay address travels *in* the token, so the joining node can
-- configure itself from what it was handed rather than asking. That
-- means it has to be decided when the token is minted, and it means two
-- tokens minted in a row must not name the same address — which is what
-- the unique index below is for.
--
-- ## Why the joining node's id is not here
--
-- It is the joining node's own, generated when it was installed, and it
-- arrives with the callback. A node that joins two hubs is one node with
-- one identity; an id allocated per enrolment would make it two.
CREATE TABLE enrolment (
    id           TEXT PRIMARY KEY,
    -- What the operator called the node they are expecting. Cosmetic
    -- and worth having: three unnamed pending tokens are unreadable,
    -- and this is the only name there is until the node arrives.
    name         TEXT    NOT NULL,
    -- Like every other secret here, storage read access alone does not
    -- authenticate.
    token_hash   TEXT    NOT NULL,
    overlay_ip   TEXT    NOT NULL,
    created_by   TEXT    NOT NULL REFERENCES account (id) ON DELETE CASCADE,
    created_at   INTEGER NOT NULL,
    -- A network credential, so it is short lived as well as single use.
    expires_at   INTEGER NOT NULL,
    used_at      INTEGER,
    -- Which node spent it. Not a foreign key: the row in `node` is
    -- written by the same call and a constraint would make the order of
    -- two writes matter.
    --
    -- It is also what makes a retry safe. A callback that arrived and
    -- whose response was lost is re-sent, and the same node presenting
    -- the same token again is the same join rather than a second one —
    -- an errand sent twice must not fail the second time.
    used_by      TEXT
);

CREATE UNIQUE INDEX enrolment_token ON enrolment (token_hash);
-- An address belongs to one enrolment. A duplicate would be two nodes
-- answering to one overlay address, which routing cannot resolve.
CREATE UNIQUE INDEX enrolment_overlay_ip ON enrolment (overlay_ip);
