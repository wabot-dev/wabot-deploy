-- Every container port a copy is reached on from another node, as a JSON
-- object from container port to the overlay port bound for it. The single
-- overlay_port reached only one, so a copy elsewhere with two named ports
-- had both names sent to the first: a store's public read went to its S3
-- port. overlay_port stays, holding the first, for an owner older than this.
ALTER TABLE replica ADD COLUMN overlay_ports TEXT;

-- What the node running a store elsewhere measured in each of its buckets,
-- sent on its report. The node that owns the store cannot ask Garage on
-- another machine, and whether a bucket is empty is what deleting it asks.
CREATE TABLE bucket_usage (
    service_id          TEXT NOT NULL REFERENCES service (id) ON DELETE CASCADE,
    name                TEXT NOT NULL,
    objects             INTEGER NOT NULL,
    bytes               INTEGER NOT NULL,
    unfinished_uploads  INTEGER NOT NULL,
    measured_at         INTEGER NOT NULL,
    PRIMARY KEY (service_id, name)
);
