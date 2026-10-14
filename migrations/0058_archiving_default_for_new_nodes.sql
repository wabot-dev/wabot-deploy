-- Keeping the write-ahead log is on for a node being installed, which is
-- what `0040` meant and did not do.
--
-- `0040` writes `off` for a node that never had the question put to it,
-- so that an upgrade restarts nobody's database. It also runs on a
-- database being created, where there is nothing to restart — so every
-- node installed since read `off`, and the default `settings::archiving`
-- describes never applied. Found on a VM node installed from scratch.
--
-- Taken back only where it was `0040` that wrote it and there is still no
-- database for it to have protected. `0040` stamps whole seconds, and the
-- console stamps milliseconds, so a choice somebody made is left alone.
DELETE FROM setting
WHERE "key" = 'wal.archiving'
  AND "value" = 'off'
  AND "updated_at" % 1000 = 0
  AND NOT EXISTS (SELECT 1 FROM "database");
