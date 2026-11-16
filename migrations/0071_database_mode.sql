-- Whether a database keeps what it holds. Only Valkey offers anything but a
-- store, and every row before this one was one.
ALTER TABLE database ADD COLUMN "mode" TEXT NOT NULL DEFAULT 'store';
