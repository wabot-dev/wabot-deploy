-- When the service was last stopped. A deployment carries the moment it was
-- asked for, and one asked for before this is a decision the stop replaced:
-- without it, the retry of a deployment that had failed, or one queued
-- behind another, started back a service stopped while it waited.
ALTER TABLE service ADD COLUMN stopped_at INTEGER;
