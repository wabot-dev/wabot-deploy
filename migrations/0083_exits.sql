-- The nodes a project leaves for the internet through, as the overlay
-- addresses the door compares (`docs/egress.md`, "Exits"), separated by
-- commas. Empty is each copy through its own node, which is what every
-- project did. On a node holding copies, or carrying them out, it is what
-- the project's owner last said.
ALTER TABLE project ADD COLUMN exits TEXT NOT NULL DEFAULT '';
