-- Errands: what an authority has asked another node to do.
--
-- ## The queue lives on the node that gives the order
--
-- Not on the node that carries it out. That is what makes this work
-- behind NAT without anything new: the node taking instructions dials
-- out, asks what is waiting for it, and reports back — the same
-- direction, over the same trusted certificate, as the callback that
-- enrolled it. Nothing has to be able to reach a private node, which is
-- the whole reason private nodes exist.
--
-- The overlay is not involved. It is a data plane: its reason for
-- existing is that the edge can reach a *container* on another node,
-- which is the next phase. Orders are control plane and travel the way
-- everything else here does.
--
-- ## Deploying stays local
--
-- An errand is an instruction, not a job. The node that receives one
-- writes its own local job for it, so there is no distributed queue and
-- no job routing — `deploy` still talks to *this* node's containerd, on
-- whichever node that is.
CREATE TABLE errand (
    id         TEXT PRIMARY KEY,
    -- Who it is for. Not a foreign key: forgetting a node deletes its
    -- row, and an errand left addressed to nobody is worth keeping —
    -- it is the record of what was asked.
    node_id    TEXT    NOT NULL,
    -- What kind of instruction. One today ('host'); the column exists
    -- because phase 4 adds 'edge' and reading an unknown kind has to be
    -- possible without a migration.
    kind       TEXT    NOT NULL,
    -- Its arguments, as JSON. Deliberately not columns: an errand's
    -- shape belongs to its kind, and a table with a column per field of
    -- every kind is a table that changes every time a kind is added.
    payload    TEXT    NOT NULL,
    created_at INTEGER NOT NULL,
    -- When the node last asked for it. An errand handed over twice is
    -- normal — a node that fetched and then died asks again — so this
    -- is the latest attempt, not the only one.
    taken_at   INTEGER,
    -- Set when the node says it finished. Both outcomes end here: a
    -- failure with its reason is an answer, and an errand that stays
    -- pending for ever because nobody recorded the failure is the state
    -- this column exists to prevent.
    done_at    INTEGER,
    -- Null when it worked. The reason otherwise, put where the person
    -- looking for it will be rather than in the other node's journal.
    error      TEXT
);

CREATE INDEX errand_pending ON errand (node_id, done_at);
