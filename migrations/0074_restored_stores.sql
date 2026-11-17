-- A store restored into a copy is made empty and brings its volume at its
-- first deployment, from a backup here or at the schedule's destination,
-- as a restored database does. Cleared once the buckets Garage holds in
-- that copy are rows again, so a later deployment does not bring it twice.
ALTER TABLE object_store ADD COLUMN restored_from TEXT;
