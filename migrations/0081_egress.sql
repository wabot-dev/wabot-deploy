-- Where a project's containers may connect outside its own network
-- (`docs/egress.md`). Every project is given anywhere, which is what it
-- had, so nothing changes until somebody takes it away; empty is nowhere.
-- On a node holding copies for another node, it is what the project's
-- owner last said.
ALTER TABLE project ADD COLUMN egress TEXT NOT NULL DEFAULT '0.0.0.0/0';
