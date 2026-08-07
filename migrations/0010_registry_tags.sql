-- What the registry actually received.
--
-- The tag index cannot be containerd's image records, which is what
-- this first tried. Those records are shared with everything else on
-- the node: `ctr images tag` writes one, a pull writes one, and the
-- registry answering `HEAD /v2/<name>/manifests/<tag>` from them says
-- "already have it" about images nobody ever pushed. A client then
-- skips the upload entirely — the push reports success, no release is
-- recorded, and nothing anywhere says why.
--
-- Sharing the *content* store is the point of this design. Sharing the
-- *namespace* of tags is not: one is bytes, addressed by their hash,
-- and the other is an assertion about what was sent here.
CREATE TABLE registry_tag (
    repository TEXT    NOT NULL,
    tag        TEXT    NOT NULL,
    digest     TEXT    NOT NULL,
    media_type TEXT    NOT NULL,
    size       INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    PRIMARY KEY (repository, tag)
);
