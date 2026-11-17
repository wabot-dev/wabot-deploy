-- The name a store's buckets are named under: a bucket answers at
-- `<bucket>.<domain>`, which is what every S3 client builds by default, and
-- Garage is told it as its root domain. Until now it was only the hostname
-- of the store's one named S3 port, and every bucket adds one beside it.
ALTER TABLE object_store ADD COLUMN domain TEXT;
UPDATE object_store SET domain = (
    SELECT port."hostname" FROM port
     WHERE port."service_id" = object_store."service_id"
       AND port."container_port" = 3900
       AND port."hostname" IS NOT NULL
     ORDER BY port."created_at"
     LIMIT 1
);

-- Public is a fact of its own now: a public bucket is read at its own name,
-- unsigned, and a name of its own for that is optional.
ALTER TABLE bucket ADD COLUMN public INTEGER NOT NULL DEFAULT 0;
UPDATE bucket SET public = 1 WHERE public_hostname IS NOT NULL;
