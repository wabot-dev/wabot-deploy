-- A node that already exists keeps what it already did.
--
-- ## Why a migration and not a default
--
-- Keeping the write-ahead log is on by default now that there is
-- pruning to bound it. For a node being installed today that is free:
-- there are no databases yet, and the first one starts archiving from
-- its first minute.
--
-- For a node that has been running, the same default means every
-- database it holds is now started with settings it was not started
-- with — and `archive_mode` is a postmaster setting, so making that true
-- means **restarting somebody's database because a default changed**.
-- Brief, and still a restart nobody asked for, arriving with an upgrade
-- they asked for something else from.
--
-- So the default is for new nodes and this is for the rest: an explicit
-- `off`, which the console shows as off and which an operator turns on
-- when they choose to. Turning it on redeploys, which is what somebody
-- clicking a switch expects; an upgrade doing it is not.
--
-- `WHERE NOT EXISTS` rather than `INSERT OR IGNORE`, so a node that has
-- already chosen — either way — is left alone. This migration is for the
-- nodes that never had the question put to them.
INSERT INTO setting ("key", "value", "updated_at")
SELECT 'wal.archiving', 'off', CAST(strftime('%s', 'now') AS INTEGER) * 1000
WHERE NOT EXISTS (SELECT 1 FROM setting WHERE "key" = 'wal.archiving');
