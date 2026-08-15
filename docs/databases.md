# Databases

Postgres first, and the plan is written so the second engine is a table
of numbers and two strings rather than a second implementation.

## What it is for

A node can already run somebody's container. It cannot run their
**database**, and the gap is not one feature — it is four things the
platform has never had:

- **A volume.** `containers::remove` deletes the snapshot, and `run`
  calls it first. Every deployment today starts from the image, which
  is right for a stateless service and total data loss for a database.
- **A memory ceiling.** Nothing writes `linux.resources` into the OCI
  spec, so every container may take the whole machine. On a one-core
  box with no swap, one Postgres with default settings is the node.
- **A role per copy.** A replica of a web service is interchangeable
  with its siblings — that is what makes the edge's round-robin
  correct. A copy of a database is *not*: one accepts writes and the
  rest follow it.
- **A path between two nodes that are not each other's authority.** A
  standby has to dial its primary. See "Two things not to discover
  late".

## A database is a service, and the difference is a row

`service` and `replica` already carry deploying, reconciling,
observing, placing elsewhere, reporting back and eviction. A database
reuses all of it:

- `service.kind` says `postgres` instead of `container`.
- A `database` row holds what only an engine has: version, preset,
  credentials, which slot is the primary.
- Slot 1 is the primary; every other slot is a read-only standby. The
  slot number already means one thing across the whole network, which
  is exactly what a role has to.

What a database does **not** get is a hostname. An edge terminates TLS
and proxies HTTP; Postgres speaks its own protocol on a TCP socket.
Refusing the hostname outright also sidesteps the trap in the paragraph
above — nothing can ever round-robin a write to a standby, because no
route ever names one.

## The presets

The operator picks how much RAM the database may have. That one choice
sets the cgroup limit **and** the engine's own arithmetic, because
setting only the first is how a database gets OOM-killed on startup:
`shared_buffers` defaults to 128 MB, which alone exceeds three of the
sizes below.

| Preset | `memory.max` | `shared_buffers` | `effective_cache_size` | `work_mem` | `maintenance_work_mem` | `max_connections` | `/dev/shm` | `max_wal_size` |
|---|---|---|---|---|---|---|---|---|
| 64 MB | 64 MiB | 16 MB | 32 MB | 1 MB | 8 MB | 10 | 16 MB | 256 MB |
| 128 MB | 128 MiB | 32 MB | 64 MB | 2 MB | 16 MB | 20 | 32 MB | 256 MB |
| 256 MB | 256 MiB | 64 MB | 128 MB | 2 MB | 32 MB | 40 | 64 MB | 512 MB |
| 512 MB | 512 MiB | 128 MB | 256 MB | 4 MB | 64 MB | 60 | 128 MB | 512 MB |
| 1 GB | 1 GiB | 256 MB | 512 MB | 4 MB | 128 MB | 100 | 256 MB | 1 GB |
| 2 GB | 2 GiB | 512 MB | 1 GB | 8 MB | 256 MB | 150 | 512 MB | 2 GB |
| 4 GB | 4 GiB | 1 GB | 2 GB | 16 MB | 512 MB | 200 | 512 MB | 2 GB |

`shared_buffers` is a quarter, `effective_cache_size` a half,
`maintenance_work_mem` an eighth and `/dev/shm` a quarter bounded to
16 MB–512 MB. The three that are not a fraction are `work_mem`,
`max_connections` and `max_wal_size`: each of those is multiplied by
something the ceiling does not know about — how many sorts a query
runs, how many clients connect, how much churn a workload makes — so a
fraction would be a formula pretending to an accuracy it does not
have.

Four things about the numbers, each of which is a mistake somebody
makes once:

- **`effective_cache_size` is half the limit, not the usual 75 % of the
  machine.** Inside a cgroup the page cache is charged to the cgroup,
  so the planner's idea of "free cache" cannot exceed what the limit
  allows — and the limit is also holding `shared_buffers`.
- **`/dev/shm` is currently a hard-coded 64 MB** in `runtime::spec`,
  which is Docker's default and the reason parallel query is the
  classic Postgres-in-a-container failure. It follows the preset here.
  A tmpfs page is charged to the cgroup that wrote it, so the limit
  still bounds it; the size is a cap, not a reservation.
- **`max_wal_size` is disk, not RAM,** and it is in the table anyway:
  the preset is the "this is a small box" knob, and a 1 GB default WAL
  on a node that has a 64 MB database is the same misjudgement one
  layer down.
- **Parallel query is off below 1 GB** (`max_parallel_workers_per_gather
  = 0`). Each worker is a process and a shm segment.

The limit is written as both `memory.limit` and `memory.swap` in the
spec, which crun turns into `memory.max` and `memory.swap.max = 0`. The
test node has no swap at all, so this only makes the intent explicit —
on a node that has some, a database quietly swapping is worse than one
that is refused the memory, because the first is invisible.

`memory_limit` is a column on `service`, not on `database`. Capping a
web service is the same operation and the same code; only the second
half of the preset — the engine's arithmetic — is Postgres's.

## Postgres, concretely

**The image comes from Docker Hub.** `docker.io/library/postgres:<major>-alpine`,
written out in full because containerd does not do Docker's
familiar-name normalisation: `postgres:17` is a reference to a registry
called `postgres`, and only the fully-qualified form resolves. The
operator can point it somewhere else — a mirror, a private copy — and
the default is what appears in the field.

**Alpine, for the size.** ~80 MB against ~150, on a node where
`deploy.sh` already takes 25 minutes. The cost is musl's collation: a
database seeded on one variant and moved to the other needs a reindex.
Nothing here moves one, and this is the note that says so before
somebody does.

**The major version is pinned and immutable.** `17` selects the tag;
minor updates arrive by re-pulling it. Changing the major is a data
migration, not an image change, and is refused with that as the reason.

**Tuning is delivered as arguments, not as a file.** The image's
entrypoint ends in `exec postgres "$@"`, so `-c shared_buffers=32MB`
reaches the server and is visible in `ctr containers info`. The
alternative — writing `postgresql.conf` into the volume — is a second
source of truth that the next deployment has to reconcile against.
This required one new field: `ContainerRequest.args`, appended to
whatever command runs, where `command` replaces it.

**`PGDATA` is a subdirectory of the mount.** `/var/lib/postgresql/data/pgdata`,
which is the documented workaround for `initdb` refusing a bind-mounted
directory that is not empty or not owned by it.

**`pg_hba.conf` is written by the node, on every deployment.** The
image appends `host all all all scram-sha-256`, and `all` in the
database column **does not match a replication connection** — so
replication with the stock file is refused with a message about no
matching entry. The node writes the whole file into a config directory
mounted read-only and passes `-c hba_file=…`; it is rebuilt from the
current rows each time, so it is convergent rather than remembered:

```
local   all           all                     trust     # the entrypoint's own socket
host    all           all   10.42.<idx>.0/24  scram-sha-256   # the project's bridge
host    replication   <repl>  10.42.0.<n>/32  scram-sha-256   # one line per standby's node
```

The entrypoint's temporary server is started with the same arguments,
so the `local … trust` line is not a convenience — without it `initdb`
never finishes.

**The replication role is not the superuser.** A node holding a read
replica already has every byte of the database on its disk, so the
superuser password would buy it nothing it does not have — except the
ability to *write* to the primary, which is precisely what a read
replica must not be able to do. The role is created by a generated
`docker-entrypoint-initdb.d` script, which runs once, at initdb. Every
wabot-managed Postgres therefore has it from birth, whether or not it
ever gets a standby; adding one later needs no SQL against a running
server, and so needs no Postgres client in this binary.

## Reaching a database

Two ways, and they answer different questions.

**From inside the project: a fixed address.** The `/24` a project's
bridge is carved from is split — `.2` to `.199` is what `host-local`
allocates from, and `.200` upward is the node's own to hand out for
things whose address is written down somewhere. The address goes to CNI
as `CNI_ARGS=IP=…`, so a redeployed database keeps the connection
string it had; left to itself `host-local` hands out the lowest free
one, which is stable only while nothing else churns.

The first version of this bounded `host-local` to the low band and kept
the high one out of its sight, on the model of the two port ranges —
where the allocators cannot collide *by construction* rather than by
both remembering to check. **That does not work, and it would have
failed on the first database ever deployed.** `host-local` matches a
requested address against its configured ranges, not against the
subnet, so an address outside them comes back as `requested IP … not in
range set`.

Both bands are therefore declared, as two ranges in one range *set*,
low first — one set yields one address, and the allocator walks its
ranges in order. The separation is by order rather than by
construction, and it holds until the low band is exhausted: 198
containers in one project on one node, at which point an automatic
allocation spills into the high band and could take an address a
database holds. Spilling beats refusing a 199th container, and the
collision surfaces as an ADD that fails by name rather than as a
container quietly answering on somebody else's address.

**From outside: a published port,** the existing `port` row with
`host_port` set. Behind a confirmation, because publishing 5432 on
`0.0.0.0` is what it says it is. One consequence to fix along the way: a
published port is a column on `service`, so two copies on one node
would both claim it. For a database the copies are not interchangeable
anyway — publishing names a *replica*.

## Read replicas

A standby is seeded with `pg_basebackup` into its own volume, then
started from it. Three decisions:

**The seed runs as a container of its own** — the same image, the
command replaced, the volume mounted, run to completion and its exit
code read. It writes its own output to a file in the volume, which the
node reads and puts on `replica.last_error` when it fails. containerd
discards a task's stdio unless it is given somewhere to put it, and a
base backup that fails silently is a standby nobody can explain.

**Skipped when the volume already holds a database.** Convergent, like
every other step: the question is "is there a `PG_VERSION` here", not
"have I done this before".

**Retried locally, with a deadline.** The primary is running before the
standby is asked for, but running is not accepting connections —
`initdb` takes seconds. The seed retries for five minutes and then
records why it gave up. It does **not** re-queue an errand: a failure
is an answer, and this codebase's rule is that retrying is something
somebody asks for.

**`standby.signal` and `primary_conninfo` come from the row, not from
`pg_basebackup -R`.** `-R` writes the conninfo into the volume, where it
becomes a remembered fact that goes stale the moment anything about the
primary changes. The node writes the signal file and passes
`-c primary_conninfo=…` on every start, recomputed from the current
rows. The password is inline in it, and therefore visible to `ps`
inside a container that already holds the whole database.

**Each standby holds a replication slot** named for its slot number,
created by the base backup (`-C -S`), with `max_slot_wal_keep_size` set
to twice `max_wal_size`. Without a slot, a standby that is down long
enough breaks and needs re-seeding. With an unbounded one, a standby
that never comes back fills the primary's disk. The bound turns the
second failure into the first, visibly.

## Across nodes

The standby dials the primary at **the primary node's overlay address
and a port bound to it** — never the container's own, for the reason
phase 7 already records: a bridge address names a different container
on every machine.

The errand carries everything, because the receiving node can look
nothing up: the image, the preset, the role, the conninfo, the
credentials, and the **peer** — the other node's public key, overlay
address and endpoint if it has one.

### The ordering problem, and why there is a dispatcher

The port the primary answers on belongs to the node running the
primary: it comes out of that node's port space, and it travels home on
a report. So when the primary is on B and a standby is wanted on C, the
node that owns the database cannot write C's errand at the moment
somebody clicks — it does not know the port yet.

So a database's errands are **recomputed rather than emitted**:
`databases::dispatch` builds what each node should be told from the
current rows, and queues it only when it differs from what that node
was last told. Called after a placement change, after a report that
moved something, and at boot. This is the same shape as the six bugs
`docs/network.md` records — derived state whose input arrives over the
network — except deliberately, and with the recomputation written
first.

Comparing against the last errand is what keeps a 15-second report loop
from queueing an errand every 15 seconds. It needs the errand to say
what it was *about*, so `errand` gains a `subject` column — which the
errands page wanted anyway.

## Two things not to discover late

**The overlay is a star, not a mesh.** `tunnel::peers` builds the peer
list from this node's `node` table, and a join writes exactly one row on
each side: the joiner learns its authority, the authority learns the
joiner. Two nodes enrolled by the same authority have never heard of
each other and have no WireGuard session. Phase 7 did not notice
because the verified topology had the owner as the edge, so every
conversation had the authority at one end.

A database whose primary is on B and whose standby is on C is exactly
the case that has no path. The fix is the peer in the errand: a node
told to hold a standby is told who to open a tunnel to, writes the row
and calls `tunnel::ensure`. Both ends need it — WireGuard requires each
side to hold the other's key — so both get an errand.

And one case has no fix in software: if neither node has an endpoint
the other can dial, there is nothing to hole-punch through and no
session can exist. Dispatch refuses that pairing with that as the
reason, rather than queueing an errand that can only ever time out.

**A push token is currently offered to whatever registry the image
names.** `send_there` mints a token for *this node's* registry and puts
it in the errand against the host in the image reference. For
`docker.io/library/postgres`, that sends a wabot registry credential to
Docker Hub. Nothing has hit it because placing a public image on
another node is not something anybody has done yet — and it is the
normal case for a database. The credential becomes optional in the
payload and is minted only when the registry is this node's own.

## What phase 4 turned out to need, and what it did not

**The peer in the errand was not needed for these two nodes.** The
overlay is a star and two leaf nodes have never heard of each other —
that stands, and it is written below. But the pair this is tested on is
*not* two leaves: Alpine joined Ubuntu, so they are each other's peers
with a live handshake. Cross-node standbys work between an authority and
one of its members with no peer distribution at all. It is leaf-to-leaf
that still has no path, and no test node is one.

**The primary's address is told, not derived.** A node holding a standby
has no row for the primary and never will, so `database.primary_endpoint`
is where the errand's answer lands. `NULL` means "work it out", which is
what the owning node does. Two ways to the same address, and the shape
`replica.node_id` already uses for "here".

**The errand is recomputed rather than emitted.** The port the primary
answers on comes out of the owning node's port space and is assigned
when the primary deploys — so an errand written at the moment somebody
clicked would carry an address that did not exist. It is rebuilt after
every deployment and queued only when it differs from the last one,
which needs `errand.subject` to say which database it is about. One node
can hold standbys of two.

**A second primary is refused at the far end.** An errand naming the
primary's slot is a mistake on the sending side, and two databases taking
writes under one name is not something anything downstream could
reconcile — not the replication slot, not the reader, not the operator.

## The `store` capability

Running somebody's container and holding somebody's data are different
things to agree to, and a node may reasonably want the first without
the second. `store` is granted and revoked on its own, like `host` and
`edge`, and the collector refuses a database errand without it.

It is **not** backfilled onto existing joins. A node that agreed to run
containers last month did not agree to keep a copy of a database, and a
migration that decided otherwise would be the wrong kind of quiet. The
two test nodes need a re-join to grant it — which phase 8 needs anyway,
since the terms screen has never been exercised.

## Phases

| | Phase | Delivers | State |
|---|---|---|---|
| 0 | Volumes | A directory per replica, mounted, surviving redeployment; removed with the service, behind a confirmation | **done**, verified on a node |
| 1 | Memory | `service.memory_limit`, `linux.resources` in the spec, `/dev/shm` following it, the preset selector | **done**, verified on a node |
| 2 | Postgres here | `database`, the kind, the Docker Hub default, tuning arguments, generated credentials, the connection string, the fixed address | **done**, verified on a node |
| 3 | Standbys here | The seed container, the replication slot, slots 2..n on this node | **done**, verified on a node |
| 4 | Standbys elsewhere | `store`, the database errand, the dispatcher, the conninfo over the overlay | **done**, verified between two nodes |
| 5 | Health | Replication lag and "this standby stopped following", which today nothing would notice | |

Phases 0 and 1 are worth having on their own: every service gets a
memory ceiling, and a volume is what half the images on Docker Hub
expect.

## What the nodes said

**Phases 0 to 2 are verified on the Ubuntu node**, on 2026-08-12, with a
256 MB PostgreSQL 17 created from the console in a project of its own.
What the machine confirmed, and each of these was an open question:

- **`memory.max` is `268435456` and `memory.swap.max` is `0`**, read out
  of the cgroup the container is in. The ceiling is real, and the
  arithmetic reached the engine: `shared_buffers` reads `64MB` and
  `max_connections` `40` from inside the server.
- **`hba_file` reads `/etc/wabot/pg_hba.conf`** from inside the server,
  so the file the node generates is the one it is actually using — and
  `initdb` finished, which is what the `local … trust` line is for.
- **`initdb` ran into the bind mount.** `PG_VERSION` says 17 in
  `…/volumes/db-test.orders/data/pgdata/`, with `PGDATA` one level below
  the mount point as intended.
- **The reserved address held, twice.** `10.42.2.200` on the first
  deployment and the same one after the container was killed and
  reconciliation brought it back.
- **The data outlived the container.** A row written at 17:50:31 was
  still there after a `SIGKILL` and a redeployment, and
  `PG_VERSION`'s creation time stayed at 17:46:14 — so the entrypoint
  found the directory and did not initialise it again. That is the whole
  of what a volume is for, and it is also what a standby will depend on.
- **The arguments are what the code says**, entrypoint intact in front:
  `docker-entrypoint.sh postgres -c listen_addresses=* -c hba_file=… -c
  shared_buffers=64MB …`.

**And one thing was wrong, in the way only a node can show.**
`CNI_ARGS=IP=…` — one of the two documented routes to `host-local` — is
refused by the chain: the `bridge` plugin parses `CNI_ARGS` into a
struct that knows `MAC=` and nothing else, and answers `ARGS: unknown
args` to anything else, so the variable never reaches the allocator it
was meant for. Every database's first deployment would have failed on
it. The address travels in `args.cni.ips` inside the config instead,
which the plugin was asked about directly: the exact address on
request, the same one again after a DEL, and `failed to allocate all
requested IPs` for one outside both ranges.

`doctor` also printed `volumes 2` on a node with two containers and no
storage at all — the container count under the wrong word. A report that
says something untrue is worse than one that says nothing.

**The Docker Hub pull works.** `docker.io/library/postgres:17-alpine`
resolved, pulled and unpacked on the Alpine node in 8.5 seconds,
anonymously, through the transfer service. It also settled three design
questions at once: the image's command is
`["docker-entrypoint.sh", "postgres"]`, so appending `-c …` is right and
replacing the command would have thrown the entrypoint away; its `User`
is root, so the entrypoint can `chown` the data directory; and it
declares 5432.

**A Postgres container with no password and no volume exits 1 in two
seconds.** Which is what a standby with an unseeded data directory will
do — the safe failure rather than a second primary accepting writes
into a copy of somebody's data.

**Neither test node can link this binary any more.** `rustc` compiling
the final crate under thin LTO was OOM-killed on the Ubuntu node at 571
MB of anonymous RSS — `--profile node` exists to avoid exactly that and
no longer does — and the Alpine node reached 100 % of its 512 MB of
swap on the same step before it was stopped. `CLAUDE.md` already lists
"building happens on the node" as open, with a static musl binary built
elsewhere as the clean way out; it has stopped being a preference. The
verification build was made with `CARGO_PROFILE_NODE_LTO=false` and
sixteen codegen units, which is the same optimisation level in pieces
that fit.

**Phases 3 and 4 are verified across both nodes** (2026-08-13), one
database owned by Ubuntu with a standby beside it and a second on
Alpine. `pg_stat_replication` on the primary shows both — `10.42.2.254`
over the project bridge and `10.42.0.4` over the overlay — `streaming`,
and the remote copy answers `pg_is_in_recovery() = t`. Both nodes run
the identical static musl binary, checked by sha256.

**Two of the four things that went wrong were only visible with two
nodes**, and both are the shape phase 7 already named — derived state
that nothing recomputes when its input arrives over the network:

- **`sslmode=disable` in the standby's conninfo**, left there from
  before `hostssl` became mandatory. The standby dialled, the primary
  refused it for lack of encryption, and *nothing said so* except the
  primary's own container log — for hours, on both standbys. The two
  functions that build a connection string now have a test asserting
  they agree with each other, because the disagreement is what hid it.
- **The copy named itself under the *holding* node's domain.** So the
  same database had a different qualified name on every machine that
  held a copy, each with a certificate matching only its own, and no
  single connection string reached it — which is the whole point of a
  qualified name. `psql` said `server certificate for "orders" (and 5
  other names) does not match host name
  "orders-ro.db-test.wabot-deploy-testing.dev.tobaw.shop"`. The owner's
  domain travels on the errand now (`0032_owner_domain`), and the
  hosts file and the certificate are both built from it. Verified after
  the fix: `sslmode=verify-full` against the owner-qualified name
  succeeds on **both** copies, writable on Ubuntu and read-only on
  Alpine.

The second one had three separate causes stacked, and the first two
were silent: `adopt` read the domain off the errand into its parameter
tuple and left the column out of the `INSERT` — a bound value nobody
uses is not a warning — and `dispatch_standbys` ran only on a
deployment, so a payload that gained a field reached nobody until
something redeployed. Reconciliation recomputes the errand at boot now;
`queue_if_changed` makes the pass free on the boots where nothing moved.

## Lowering the preset breaks the standby, and nothing local could tell

Reported from the node (2026-08-15): a 256 MB database moved to 64 MB in
settings. The primary restarted at the new rung without a complaint; the
standby beside it aborted recovery and stayed down.

```text
FATAL:  recovery aborted because of insufficient parameter settings
DETAIL: max_connections = 10 is a lower setting than on the primary
        server, where its value was 40.
```

The rule is not the one it reads as. A standby's `max_connections` has
to be at least the primary's value **recorded in the log it still has to
replay**, not the primary's value now — and the record that says 40 is
sitting in the log ahead of it, so it cannot get past the thing that
would tell it the primary is at 10. The directory is finished, and no
restart, retry or wait moves it.

Three things about the shape of this, because it is not one bug:

- **The ladder was doing exactly what it was designed to.** `tuning`
  gives the 64 MB rung ten connections and the 256 MB rung forty. Both
  copies get the same number from the same row, so they are always
  equal — it is the *transition* that is impossible, not the state.
- **Only downwards.** A standby may run above the primary and never
  below it, so raising the preset was always safe.
- **The primary is fine and says nothing.** It is a fresh start with a
  new value; the write-ahead log gets a parameter record and life goes
  on. Everything that is wrong is inside the copy.

What the node does now is ask the volume rather than work out what
changed: `.wabot-max-connections` beside the data directory records what
this copy was last started at, and a standby whose ceiling has come down
has its `pgdata` thrown away and copied from the primary again — which
is already at the new rung, so the new copy has no such record in it.
Asked of the thing rather than of the history, because the machine
holding a copy may have been switched off when the operator lowered the
rung and an errand that arrives days later has to reach the same answer.

A volume with no record is one seeded before this existed: left alone,
and given a record from then on. Copying every standby on the node once
on upgrade is a worse answer than the one case somebody has reported.

**No test could have found this.** It needs a primary with a standby
following it, a preset change, and a second start — three node runs, and
the failure is inside PostgreSQL's own recovery check.

## What has not been run on a node

Everything above passes `cargo fmt`, `cargo clippy -D warnings` and 690
tests. What is left unverified:
- **The published port.** A database reachable from outside the node is
  a `port` row with a `host_port`, and no database has had one.
- **Reading a remote pool from a third node.** A node that holds no
  copy has no address for one: `orders-ro` resolves inside the project
  that holds it and nowhere else. That is naming's phase 4.
- **Failover of any kind.** Nothing promotes, and nothing notices a
  standby that stopped following — phase 5.
- **The errand a new node sends an old one.** A `host` errand for an
  image from a registry that is not this node's own now omits the
  credential, and a node old enough to require it answers "that is not
  a host errand". Both nodes are current, so nothing exercises it.

## Open, and deliberately not decided yet

- **No failover.** `database.primary_slot` exists so that promoting one
  is a row rather than a migration, and nothing writes it. A promotion
  that the node performs on its own is a way to end up with two
  primaries.
- **No backups.** A volume on one node is not a backup, and a read
  replica is not one either — a `DROP TABLE` replicates in
  milliseconds.
- **No password rotation.** The replication role's password is set at
  `initdb` and there is no SQL path to change it without a Postgres
  client in this binary.
- **No connection pooler.** `max_connections` on the 64 MB preset is 10,
  and something has to be the thing that says so.
- **Reading replication state needs a Postgres client.** Phase 5 is
  where that dependency gets argued for; until then the console shows
  the container's state, which does not know whether replication is
  following.
