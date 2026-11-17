# Buckets

Object storage with the S3 protocol, made from the console the way a
database is: a service with a name, a volume and a ceiling, whose image,
configuration and credentials the node writes.

## What it is for

An application that stores files — uploads, generated reports, a
static site, a backup it takes of itself — needs somewhere that is not
its own container, whose filesystem a deployment replaces. Today it has
two answers on this platform and neither is one: a volume, which pins
the service to one node and speaks no protocol a library knows, or an
account with somebody else's object store.

S3 is the protocol every library already speaks. A bucket here is
reached with the client the application already has, by changing an
endpoint and two keys.

## What was decided, with Jorge

Asked for on 2026-11-16, and decided the same day.

- **An engine in a container, managed like the databases.** Not in the
  binary: an S3 server inside wabot-deploy was offered first and turned
  down, because storing tenants' objects is not this program's job any
  more than running their Postgres is. The node runs the engine, writes
  its configuration and converges it to the rows; the engine stores.
- **Garage.** See "Why Garage" for what else was looked at.
- **An endpoint per store.** A store is a service with its own HTTPS
  name, and its buckets are addressed under it, path-style.
- **One key, one bucket.** A key opens a single bucket, to read and
  write or only to read. Revoking one touches nothing else.
- **The first version has** objects, listings, multipart uploads, batch
  delete and copy — what any client does without being asked — and
  also presigned URLs, CORS, public read and expiry. One copy, on the
  node that owns it, with a nightly backup.
- **The licence is accepted** (see "The licence"). If an adopter cannot
  run AGPL software, the answer is that their nodes do not run it — a
  switch — not that it is hidden from them.
- **The switch is off by default.** Nobody runs AGPL software without
  having decided to.
- **Every bucket has a maximum size**, and a store's buckets add up to
  no more than its disk ceiling.
- **A bucket is deleted only when it is empty.**
- **The region is `us-east-1`.**

## Why Garage

What was compared on 2026-11-16, for a node with one core and 1 GB:

| | Licence | At rest | Why not |
|---|---|---|---|
| **Garage** 2.5 | AGPLv3 | ~50 MB reported | — |
| MinIO | AGPLv3 | — | Images and binaries stopped in October 2025, maintenance mode in December, repository archived in 2026. |
| RustFS 1.0 | Apache-2.0 | 90–250 MB idle | Grows under load and does not give it back, and nothing bounds it: no setting to tell it a number ([#1637](https://github.com/rustfs/rustfs/issues/1637) asked and was closed). A single drive with 4,700 objects was killed at 1 GiB and at 1.5 GiB after listing, copying and deleting ([#9145](https://github.com/rustfs/rustfs/issues/9145), open). Under a ceiling that is a store the kernel kills mid-upload. |
| SeaweedFS | Apache-2.0 | ~250–300 MB | Master, volume server, filer and gateway in one process: the floor would be half of a small node. |
| versitygw | Apache-2.0 | small | Its flat-file accounts are documented as for testing, it has no quotas, and expiry was not found. |
| s3s, in a program of our own | ours | ours | The protocol is the crate's; storage, listings, multipart, CORS, expiry and every client's quirks would be ours to find one at a time. Kept as the way out if the licence ever has to change — see "The licence". |

**Garage covers each decision above one to one**: permissions per key
and bucket (read, write, owner), presigned URLs, CORS, lifecycle with
`Expiration` and `AbortIncompleteMultipartUpload` — exactly what expiry
asks for — website access for public reads, quotas per bucket, and an
admin API to drive all of it. It has no versioning and no object lock,
and answers 501 to what it does not implement.

**And it has settings to measure a ladder against**, which is what made
RustFS impossible: `block_ram_buffer_max` (256 MiB by default; when full
it slows the client instead of holding more), `block_size` (1 MiB) and
`block_max_concurrent_writes_per_request` (3). None is a hard cap — the
first does not count per-request buffers — so the ceiling still has to
be measured, as Valkey's was.

## The licence

AGPLv3, and what that does and does not ask of this project:

- **wabot-deploy stays ours.** Garage runs in its own container and the
  node talks to it over HTTP — the S3 API and the admin API. Two
  programs talking over a socket are two works.
- **We do not distribute it.** The node pulls `dxflrs/garage` from
  Docker Hub when it deploys, as it pulls `postgres` and `valkey`.
- **Offering it to tenants, and charging for it, is allowed.** That is
  what Redis 7.4's licences forbade.
- **A patched Garage would have to be offered in source** to whoever
  reaches it over the network — the tenants. Publishing the fork is the
  whole of it, and nothing here patches it.
- **Some companies refuse any AGPL**, without reading further. That is
  the reason `docs/databases.md` gives for Valkey over Redis 8, and it
  was decisive there because Valkey cost nothing. Here nothing permissive
  weighs the same.

So two things are built for the day an adopter cannot run it:

- **A switch per node** (see "Off until turned on"). A node with it off
  never pulls the image and never offers the form.
- **The engine behind a narrow edge.** Everything outside `policies::garage`
  and the convergence talks about stores, buckets and keys, never about
  Garage, so a permissive engine — s3s in a program of our own — could
  replace it without the console, the rows or the routes noticing.

The licence notice is never hidden. `doctor` and the switch name it.

## A store is a service, and the difference is a row

`service` and `replica` already carry deploying, reconciling, restarts,
logs, the ceiling, the volume and the backup machinery. A store reuses
all of it:

- `service.kind` is `garage`.
- An `object_store` row holds what only the engine has: its version, the
  secret Garage's RPC wants, and the admin token — the only credential the
  node holds for it.
- `bucket` rows are the buckets, `bucket_key` rows the keys, each
  belonging to one bucket.

**Unlike a database, TLS is at the edge.** `docs/naming.md` put TLS in
the server because Postgres speaks its own protocol on a socket. S3 is
HTTP, and the edge already terminates TLS for HTTP, checks the allowlist
and proxies — `edge/proxy.rs` keeps the `Host` (an HTTP/2 authority
included), and streams the body without holding it, which is what a
SigV4 signature needs. So the store's name is a port row with a
hostname, routed to Garage's S3 port, like any container's.

**`is_managed()` has been answering "is a database".** About twenty-five
places ask it, and for most of them the honest question was whether the
service has a `database` row: `deploy/routing.rs` drops the HTTP route
of everything managed, which is right for Postgres and is exactly the
route a store needs. Phase 2 reads each of them and asks the question it
meant by name — the node writes this service's image, or this service is
a database — so a store is neither treated as Postgres nor left out by a
check that never thought of it.

## The rows are the truth, and Garage is derived

The same rule as `pg_hba.conf` and Valkey's `aclfile`: what the node
writes is rebuilt from the rows on every deployment, so it converges
rather than remembers.

**The configuration** is `garage.toml`, written into the config
directory and mounted read-only. The command is the image's binary with
`-c` naming it, and `server --single-node`, which lays out a one-node
cluster without the layout dance. The secrets are files beside it
(`rpc_secret_file`, `admin_token_file`), never the environment and never
the command line: `ctr containers info` prints both.

**Garage's own state is driven through its admin API**, on the
container's address and port 3903, from the node and never through the
edge. After every deployment, and after every change from the console:

1. Each bucket row exists, with its name as the global alias
   (`CreateBucket`), and a public one with its public name as a second
   alias (`AddBucketAlias`, `RemoveBucketAlias`).
2. Each key row exists in Garage. A key is minted by Garage
   (`CreateKey`) and the node keeps what it returns. **`ImportKey` is
   only for giving a key back** to a store that lost it — a restore, a
   volume rebuilt — which is the use Garage's own specification limits
   it to; it says not to use it to choose identifiers.
3. Each key has exactly its permissions on its bucket (`AllowBucketKey`,
   `DenyBucketKey`), and **any key Garage holds that no row names is
   deleted** (`DeleteKey`). A store's keys are the rows', and a key the
   console revoked must stop working whether or not the deployment that
   revoked it finished.
4. Quotas, website access, CORS and expiry per bucket, in one
   `UpdateBucket`. The first version of this plan said CORS and expiry
   were S3 calls only, signed with a key of the node's that was owner on
   every bucket; the v2 admin API takes `corsRules` and `lifecycleRules`
   directly, so that key was never made. Measured on the VM: an
   `UpdateBucket` with both, read back by `GetBucketInfo` exactly as
   sent, and enforced at the edge — a preflight from a listed origin and
   from a `https://*.` one answered, one from elsewhere refused.

**A tenant's key is never owner.** An owner can change CORS and
lifecycle through the API, and the next deployment would overwrite what
it wrote: two writers of one setting is the drift every derived thing in
this project has been rebuilt to avoid. Configuration is the console's.

**A key must work the moment its page says so.** Creating one runs the
convergence on the running store in the same request, and the page
reports what Garage answered, not what the row says.

## Addressing

- **The endpoint is the store's name**, derived as any service's —
  `<store>.<project>.<node's domain>` — and editable, with its
  certificate chosen by `edge::policy` as every name's is.
- **Path-style**: `https://<name>/<bucket>/<key>`. The page shows the
  setting each common client needs for it (`forcePathStyle: true` in the
  JavaScript SDK) — measured per client in phase 3, not assumed.
- **Virtual-hosted style is later.** `<bucket>.<name>` would need a route
  and a certificate per bucket, or a wildcard, whose DNS-01 path has not
  run against a real authority (`docs/dns.md`, phase 7).
- **A region is part of every signature.** `us-east-1`, so a client that
  was never told one still signs for the right one, rather than Garage's
  own default, `garage`, which every client would have to be given.
  Measured per client in phase 3.
- **A bucket name** is S3's rule with one thing taken away: 3 to 63
  characters, lowercase letters, digits and hyphens, starting and ending
  with a letter or digit. **No dots**: a dotted name cannot be a single
  label under the store's name, which is what virtual-hosted style will
  need. Unique within the store, not the node — the store is the
  namespace, which is what an endpoint per store bought.

## Public read

Garage serves anonymous reads only on its **website endpoint**, port
3902, choosing the bucket by `Host`. So a public bucket has a name of
its own routed there — derived `<bucket>.<store's name>`, editable, so
`assets.example.com` can be one — and reads by key need no signature.
What it does not do, and the page says so: list. The website endpoint
answers objects, not directories, and serves `index.html` for a path
that ends in `/`.

**One port, several names.** Every public bucket of a store is a name
on the store's port 3902, and the port table declared a container port
once per service (`port_service_container`). Migration 0073 keeps that
for a port with no name and lets named ones share a container port: a
name is still one service's, by its own index. Garage finds the bucket
by the full name, because no `root_domain` is configured, so the public
name is also an alias of the bucket there — which means it has to pass
Garage's rule for a bucket's name: at most 63 characters, and refused
with a sentence when it would not.

Measured on the VM through the edge: an object read with no signature,
`index.html` for a path ending in `/`, a listing answered 404, a write
refused, the S3 endpoint still asking for a signature, and after
"Make private" the name not answering and the alias gone from Garage.

Turning public read off takes the name away and denies website access,
in that order. The allowlist applies to the public name as to every name.

## CORS

Per bucket, one rule: the origins somebody lists, one per line;
`GET`, `HEAD`, `PUT`, `POST` and `DELETE`; any header; `ETag` exposed, so
a browser uploading in parts can complete. `*` is allowed and the form
says what it means. Without it, a page that uploads with a presigned URL
fails in the browser and works in `curl`, which is the report this
exists to prevent.

## Expiry

Per bucket, two numbers, both optional: delete objects after N days, and
abort an upload that was never completed after N days. **The second is
on by default, at seven days**: an abandoned multipart upload holds disk
and appears in no listing, so without it a store fills with what nobody
can see.

Garage's lifecycle worker runs once a day, at midnight UTC. The page says
"within a day of", not "at".

## Sizes

**Memory is measured, not chosen**, on the VM node under each rung, the
way Valkey's ladder was. What phase 2 has to answer:

- Where `block_ram_buffer_max` goes under each ceiling. Its default,
  256 MiB, is more than four of the rungs.
- Whether writes need `memory.high`. Block writes are page cache, charged
  to the cgroup, and dirty pages nothing has started writing back are
  what killed a 64 MB Valkey store. That lesson is exactly this case.
- SQLite or LMDB for the metadata. SQLite in WAL mode survives being
  killed, which a container under a ceiling sometimes is; LMDB is what
  Garage recommends for speed. Measured for both.
- Whether compression (zstd, level 1 by default) is worth its CPU on a
  one-core node.

**Disk:** every bucket has a maximum size, which becomes its Garage
quota, and the sum of a store's buckets has to fit its disk ceiling. Garage has quotas per bucket and none per store, so the sum is
the node's admission rule — refused with the numbers, as the resources
form refuses — and a full bucket answers its client with an S3 error
instead of filling a disk that other services share.

## Backups and restore

**Nightly, like a database's.** Garage's data directory is two things:
the metadata, a database, and the blocks, content-addressed files that
never change once written. Copying the metadata while it is written
risks a copy that does not open, so the backup asks for a snapshot first
(`CreateMetadataSnapshot`, node `self`) and copies the snapshot and the
blocks. A block is written before the metadata that names it, so blocks
copied after the snapshot hold every object the snapshot knows — as long
as nothing deletes one in between — and Garage waits ten minutes before
deleting a block nothing names (`BLOCK_GC_DELAY`, 600 s in
`src/block/manager.rs`). That is the window a copy has: a store whose
blocks take longer than that to copy can come back with an object whose
block is gone.

The copy carries the snapshot **in place of** the live `meta/db.sqlite`,
drops its `-wal` and `-shm`, and leaves out the snapshots directory, which
it would otherwise carry twice; the manifest says `Consistent`. A store
not running here gets a plain file copy, marked `CrashConsistent`,
because nothing asked Garage. Measured on the VM: the copied metadata
passes `PRAGMA integrity_check`, and a Garage started on the copy answers
the bucket, its object and its key.

**A restore of the node puts the volume back like any other**, and the
first convergence gives back, through `ImportKey`, any key the rows hold
and Garage does not.

**One store restores into a new one beside it**, as a database does, from
the Buckets tab — and **without its keys**, decided by Jorge on
2026-11-16: two stores never open with one credential, so a key that
leaked from the original does not open its copy too. What it does:

- The form finds the backup the way a database's restore does, through
  the same `find_copy`: one at or before the moment, here first and then
  at the schedule's destination. A manifest entry now names the store's
  service, so a later store with the same name is not given this one's
  copy.
- `CreateStore` makes the copy with the original's memory, disk and
  allowlist, and a name and address of its own. The row carries
  `restored_from` (migration 0074).
- At its first deployment, before Garage starts, the volume is copied
  beside its directory and renamed into it, so a deployment killed half
  way leaves no metadata to be mistaken for a finished copy. A backup at
  the destination is fetched first, with `fetch_dir`, as for a database.
  The original's `peer_list` is left behind: it names the original's
  address, and the original is usually still there.
- Once Garage answers, every bucket it holds becomes a row with its
  quota, CORS origins and expiry read back from `GetBucketInfo`, and
  `restored_from` is cleared. None is public, because the public name is
  the original's. The convergence that follows deletes every key no row
  names, which is all of them.

Measured on the VM with `shop/files`: the copy came up on a new RPC
secret over the original's `node_key`, with both buckets, their rules,
and `uploads`' two objects. The original's key was answered "No such
key" by the copy and still listed the original. A read-only key made on
the copy read `hello.txt` and was refused a write, and was itself
unknown to the original. The public name stayed with the original, and
the copy's log shows no attempt to reach it. A moment before every
backup and a name already taken were each refused with a sentence, and
left nothing made.

**What it costs**: a backup is the whole of what is stored. The Backup
tab already says the size of what it sends.

## Off until turned on

A node setting, **off by default**: with it off the node never
pulls Garage's image, the form for a store does not exist, and a store
row from a backup or another node is refused with a sentence. Turning it
on is a sentence that names the engine and its licence.

A node setting rather than a capability: capabilities are favours one
node grants another, and this is a decision about the machine. "Runs on"
another node will need that node to have it on, and asks it when it
comes.

## Refused, with a sentence

- A store on a node whose switch is off.
- A second copy.
- A bucket name that is not one (with the rule), or one the store
  already has.
- A bucket whose maximum would take the store past its disk ceiling,
  with the numbers.
- **Deleting a bucket that holds objects.** Garage refuses it too
  (`DeleteBucket` answers 400), and emptying a large bucket from the node
  is a job, not a request. The page says how many objects it holds.

## Phases

1. **The engine on paper.** Written: `Kind::Garage`, migration 0072
   (`object_store`, `bucket`, `bucket_key`), the entities, `BucketName`,
   `CorsOrigin`, and `policies::garage` — the configuration it renders,
   with each claim above stated as a test. Nothing offered anywhere.
   `clippy` calls all of it dead until phase 2 calls it, so the two land
   together.
2. **A store on the VM.** The `is_managed()` questions split by name.
   Made from the form with scripting off, deployed with its
   configuration and secrets, routed over HTTPS by its name, answering
   `aws s3 ls` path-style. The ladder measured, and the switch.
3. **Buckets and keys.** Their pages, the convergence, a key shown once.
   A read-only key refused a write. Measured with aws-cli, boto3, the
   JavaScript SDK and rclone: a plain PUT, a GET with a range, a
   multipart upload of hundreds of MB through the edge, a listing with a
   delimiter, a batch delete, a copy, a presigned GET and PUT.
4. **CORS and expiry.** A browser uploading with a presigned URL.
5. **Public read.** Its name, its route, an object read without a
   signature, a listing refused.
6. **Backup and restore.**

**Where it stands (2026-11-16).** All six written; the gate passes.
Measured on the VM node with aws-cli through the edge: a store made from
the form with scripting off, a bucket, a read-write and a read-only key,
a PUT, a listing, a GET, the read-only key refused a write, a presigned
GET, an unsigned GET refused, the quota refusing a 2 MB object in a 1 MB
bucket with Garage's sentence, CORS and expiry as above, deletion refused
for a bucket holding an object, a revoked key stopping at once, public
read as above, `memory.high` at 224 MB under a 256 MB ceiling (11.5 MB in
use idle), the backup, and a restore into a copy ("Backups and restore").
**Not measured yet**: a real browser, anything with ACME on, and a
restore from a backup that is only at a destination.

**The clients, measured the same day** (aws-cli 2, boto3 1.35.71,
`@aws-sdk/client-s3` 3.1170.0, rclone 1.68.2), each through the edge:
a PUT, a range GET returning exactly its bytes, a multipart upload (300
MB from aws-cli in 8 s, 50 MB from boto3, 40 MB from the JavaScript SDK,
30 MB from rclone) coming back byte for byte, a listing with a delimiter
answering its prefixes, a copy, a batch delete, a presigned GET and PUT.
Each one without path-style fails exactly as the page says it will, by
asking DNS for `<bucket>.<store>`. The node's own memory went from 27 to
30 MB while 300 MB went up and came back down; Garage's container
touched 226 MB, its `memory.high`, and was not killed.

Two clients need a second setting, and both are about presigned URLs, so
the page says them beside path-style:

- **boto3 signs a presigned URL with SigV2** (`AWSAccessKeyId`,
  `Signature`, `Expires`) unless given `signature_version="s3v4"`.
  Garage reads only SigV4 and answers "does not support anonymous access
  yet" — 403, for a GET and a PUT alike.
- **The JavaScript SDK puts a CRC32 into a presigned PUT**
  (`x-amz-checksum-crc32` and `x-amz-sdk-checksum-algorithm`, signed),
  computed over the empty body it had when presigning. Every upload to
  that URL is then refused with `InvalidDigest`.
  `requestChecksumCalculation: "WHEN_REQUIRED"` leaves it out.

What a browser would see, with `curl` sending its `Origin`: the
preflight for a presigned PUT answered for a listed origin, and the PUT
and one part of a multipart upload each answered with `etag` and
`access-control-expose-headers: ETag`, which is what completing the
upload in a browser needs.

Every sentence a phase adds has its Spanish in that phase. "Garage" is a
product name and "bucket" is the word S3 users use in both languages;
neither is translated.

## Later

- A store whose one copy runs on another node, as "Runs on" does for a
  database.
- More than one copy. Garage replicates between its own nodes, and the
  overlay is a network it could use; that is a design of its own.
- Virtual-hosted style.
- Usage per bucket on the page — `GetBucketInfo` answers bytes, objects
  and unfinished uploads.

## Not decided yet

Nothing that is Jorge's. What is open is measured, and "Sizes" lists it.
