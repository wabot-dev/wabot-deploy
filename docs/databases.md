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

**The published port is verified** (2026-08-13): a port from the free
range on the Ubuntu node, `sslmode=verify-full` against the database's
own qualified name from the Alpine node over the internet, on a
certificate a public authority signed. It was in the list below for a
week after it had been done.

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

And the obvious way out of it did not work either. Jorge took the copies
to one and back to two, which should be a new standby and a new copy of
the primary, and got the identical failure back: **the row went and the
directory stayed**. A slot filled again gets the same container id, so
the volume left behind was adopted rather than seeded — the second
standby *was* the first one's data directory.

Not deleting a volume is the right rule for the one copy of something,
and it is exactly backwards for a standby: that directory holds a copy of
the primary and nothing of its own, and adopting a stale one fails in two
ways that both read as the database being broken — the parameters above,
and a replication slot dropped meanwhile whose write-ahead log the
primary has long since recycled. So `forget_replica` takes the directory
with the row, for a standby and never for the primary, and both places
that drop a replica go through it: the placement form here, and a node
being told elsewhere that a slot is no longer its.

## What has not been run on a node

Everything above passes `cargo fmt`, `cargo clippy -D warnings` and 771
tests. What is left unverified:
- **Reading a remote pool from a third node.** A node that holds no
  copy has no address for one: `orders-ro` resolves inside the project
  that holds it and nowhere else. That is naming's phase 4.
- **Promotion.** Nothing writes `database.primary_slot`. Noticing a
  standby that stopped following is done — see below — and promoting one
  is deliberately still a person's decision.
- **A preset lowered while a standby follows.** The volume carries its
  ceiling now and a copy whose ceiling comes down is seeded again — but
  what has run on a node is the seeding, reached by removing the copy
  and adding it back. The automatic path has never fired.
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


## Phase 5: noticing a standby that stopped following

A standby can be up, healthy by every measure this node had, and no
longer replicating. The container runs, the process answers, memory and
CPU and disk all read normally — and the data is frozen at whatever
moment it stopped. Somebody reading from it gets answers, and they are
old, and nothing said so.

### Asked of the primary, and about slots

`pg_replication_slots`, not `pg_stat_replication`. The second lists
standbys that are **connected**, so one that stopped following is simply
absent — and absence cannot be told from a standby nobody ever created.
A slot is a row either way and carries `active`.

It also carries the consequence: `restart_lsn` says how much
write-ahead log the primary is holding for that slot. An inactive slot
makes the primary keep WAL until `max_slot_wal_keep_size`, after which
the slot breaks and the standby has to be seeded again — so this is the
number that distinguishes "reconnect it" from "it is already too late".

### The image's psql, not a client in the binary

The dependency argued for twice and deferred twice, decided by
measurement:

| | |
|---|---|
| the image's `psql`, one container per ask | **134 ms**, measured on the node |
| `tokio-postgres` in the binary | **21 new crates**, fifteen of them to do SCRAM-SHA-256 |

Three things settled it, and the first is that **this node already does
exactly this**: `seed_standby` runs the same image with `pg_basebackup`
instead of the server, and the traps are already written down. The
second is that the client version then always matches the server's,
where a pinned crate and a Postgres 18 age apart. The third is the
promise at the top of this document — that a second engine is a table of
numbers and two strings. A Postgres client in the binary helps MySQL not
at all; MySQL's client comes in MySQL's image.

That also corrects an entry below: "no SQL path to change [the
replication password] without a Postgres client in this binary" is
wrong. There is one, by this road, and `pg_promote()` for a future
promotion is the same.

### Three states, because NULL is one of them

`replica.following` is NULL until somebody has asked. A node that has
just started, or one whose primary is on another machine, has no
opinion — and rendering "not following" for a copy nobody has asked
about would be the page inventing an outage. Only `Some(false)` shows
the badge.

### What it does not do

**Only the primaries on this node.** A database whose primary lives
elsewhere is that node's to ask, and the answer would have to travel —
the shape reporting already has, and where this goes when promotion
exists. Today every primary is on the node that owns the database,
because nothing moves one.

**And it still does not promote.** A node that decides on its own that a
machine it cannot reach is dead will eventually decide it during a
partition, and two primaries is worse than an outage somebody was told
about.


## Point-in-time recovery, verified on the node

A database can be taken back to a moment: `wabot-deploy restore orders
--to "2026-08-17 00:16:45"`. Verified end to end on the Ubuntu node —
three rows created, a base backup taken, the rows deleted, and the
restore brought them back while the original stayed empty and serving.

Postgres's own account of it:

```text
redo starts at 0/18000290
restored log file "000000010000000000000019" from archive
recovery stopping before commit of transaction 767, time 00:17:51
selected new timeline ID: 2
database system is ready to accept connections
```

### A copy beside the original, never the original rewound

Rewinding is irreversible — everything after the chosen moment is gone,
which is exactly what somebody hunting one dropped table does not want —
and it leaves the read-only copies ahead of their primary, needing to be
seeded again. A copy costs disk and nothing else.

`recovery_target_action = promote`, so the copy finishes recovery and
opens for writes on a new timeline. The default is `pause`, which for a
database being restored into a console is a server that comes up
refusing connections with no obvious way out of it.

### What it is made of

| | |
|---|---|
| the anchor | `pg_basebackup -Ft -z`, taken from a read-only copy when there is one |
| the log | the primary's own `archive_command`, gzipped, one segment a minute |
| the reach back | the oldest base backup kept |
| the reach forward | the last archived segment, which is **not** "now" |

Both directions are on the database's page, and three of the four
answers it can give say "not what you think". The one worth its own red
badge is log arriving with no base backup to replay it onto: the archive
fills, the disk goes down, every reading is normal, and it recovers
nothing.

### Four things the node taught, and no test could

- **The archive directory has the TLS key's fault.** Made by the node as
  root, written by a server that is not. `archive_command` reported exit
  code 1 and nothing else — Postgres says the command failed, never what
  the shell thought of it.
- **`archive_timeout` is a floor.** The switch is made by the
  checkpointer when it next wakes: measured at two to three minutes for
  a value of sixty seconds. A horizon must be the time of the last
  archived segment rather than "a minute ago".
- **An idle database archives nothing**, and that is correct — the
  timeout forces a switch only when there has been activity.
- **A backup taken with `--out` elsewhere was invisible to `restore`**,
  which answered "no backup on this node was taken before that moment".
  True, and read in the middle of a recovery as "you have nothing", by
  somebody holding it in their hand. `--from <path>` names one.

### What it still does not do

**Restore a node**, as opposed to a database. The identity question —
same node or new one — is written in `docs/roadmap.md` §5 and the
manifest already carries what it needs. **And no remote destination
yet**: a backup on the same disk protects against nothing that has ever
happened to a disk.

## A database with no read replica could not be backed up

Found on `node-1` on 2026-08-22, by running `wabot-deploy backup` there
after the console gained a backup schedule. `pg_basebackup` was refused:

```
FATAL:  no pg_hba.conf entry for replication connection
        from host "10.42.2.1", user "wabot_replication", SSL encryption
```

**`10.42.2.1` is the bridge gateway**, and that is where the node's own
work arrives from: `backup` runs `pg_basebackup` in the host's network
namespace, so it has no bridge address of its own. The `hostssl
replication` lines were built from the standbys — every node holding one,
plus this project's `/24` when a standby runs *here* — and a database with
no read replica produced **no replication line at all**. So the one
database on that node had never been in any backup taken there.

Three things about the fix, in `deploy::replication_sources`:

- **The gateway is always admitted**, standby or not, because the node
  takes base backups of every managed database it runs.
- **A `/32`, not the subnet.** The gateway is an address no container can
  be given, so admitting it grants nothing to anything on the bridge. The
  subnet is still added for a local standby, whose *streaming* connection
  comes from its own container address.
- It is a pure function now, so
  `the_node_may_take_a_base_backup_of_a_database_with_no_standby` can ask
  it directly. The composition was fifteen lines inside `prepare_engine`
  and nothing could reach it.

**The second fault is the one that made the first invisible**: `backup`
compared managed services against volumes copied, printed "1 of 1
database(s) were NOT copied", and then **reported success**. A schedule
whose whole point is that nobody reads the narration would have written
"last good" on a backup holding no database, every night. `back_up_engines`
now returns what it copied *and* what it missed — a database running on
another node is `Ok(None)` and not a miss, which is why counting could
never have been right — and a run that missed one comes back as an error:
the row, the Backup tab, the attention card and `doctor` all say so, and
the command exits 1.

`pg_hba.conf` is rewritten on every deployment, so the fix reaches a
database when that database is next deployed.

**Verified on `node-1` the same night.** Redeploying `coffe-db` wrote
`hostssl replication wabot_replication 10.42.2.1/32`, and the next
backup — taken from the console's own button — holds
`volumes/coffe-store.coffe-db/{backup_manifest, base.tar.gz,
pg_wal.tar.gz}`: 4.6 MB, and `doctor` reads `last good … 1 volume(s)`
where it had read `LAST FAILED`. A database that had never been in a
backup on that node is in one.

## Going back, from the page that says you can

The window card told an operator "any moment between these two" and there
was nowhere to go with it: `restore` was a command, so recovering meant an
SSH session in the middle of the thing being recovered from. Asked for by
Jorge, on the node, looking at that card.

The form is on the database's own page, under the window, and what it does
is what the command always did:

- **One database, into a copy.** Not the node's backup applied whole. A
  new service in the same project, the same credentials, its own name —
  and the original still running and still holding its own data. That is
  what makes it something somebody can press: nothing is taken away, and
  comparing the two is most of what a recovery is.
- **The moment is typed, in UTC**, in the spelling the window prints and
  `parse_target` reads. Not a `datetime-local` control: that submits the
  browser's own zone, and a restore an hour from where somebody meant is
  the failure this whole area exists to prevent. Empty means "as far as
  the archived log goes".
- **The form is absent when it could only refuse.** It appears when a
  backup on this node holds a copy of *this* database — which the window
  cannot answer, because the window measures the archived log and a log
  with no base backup under it reaches nothing.
- **The deployment is queued by the handler.** `restore_into` writes what
  to unpack and where to stop; the unpacking happens at the first
  deployment. An operator who pressed Restore asked for a database, not
  for a service they then have to notice and start.
- `restore_one` is the one implementation and both doors are thin over it.
  Its refusals are **typed**, not strings: two of them have a way out that
  only a terminal has — `--from <path>` for a backup that has been moved —
  and the console must not print a sentence about a flag it does not have.

**Not verified on a node yet.** The rows are covered by console tests,
including that a refused restore leaves no half-made database; what has
not run on a real machine is the unpack-and-replay that follows.

### And when the backup is not on the node any more

Asked straight after: with a schedule and a destination, **nothing is
here** — the local staging copy is removed the moment the transfer
succeeds, which is the whole point of having sent it. So the machine that
had been backing itself up all week would have told its operator it had no
backups. Three parts to the answer, and one of them was a bug already in
the tree:

- **The search has a second place to look.** `restore_one` reads this
  node's own root first, then whatever the plan names — a bucket, another
  machine, or a second disk here. The comparison is against the root, not
  `is_remote`: "somewhere else" is the question, and a local second disk
  is somewhere else.
- **The manifest answers "does it hold this database".** Not the disk:
  `back_up_engines` writes one entry per volume it copied, so a database
  it could not copy is absent from that list — and that makes the question
  answerable of a bucket for the price of a listing and a few hundred
  bytes per backup. Downloading one to find out whether it is the one is
  the cost this path exists to avoid.
- **The download belongs to the deployment, not to the form.** The row
  records where the volume comes from (`s3://…/volumes/<container>`), and
  `prepare_engine` fetches it into `data_dir/fetched/<new container>`
  before the unpack. A form does not wait minutes on a bucket, a fetch
  that fails lands on the replica's row like every other deployment
  failure, and `fetch_dir` is convergent, so a retry does not pay for the
  bytes twice. `discard` removes the directory with the copy.
- **`base_for` grew the container filter** and lost nothing: it is one
  function over `(location, manifest)` pairs, used for the local root and
  for the destination, generic in the location because one is a path and
  the other is a bucket key.
- The two refusals are kept apart on purpose. "No backup was taken before
  that moment" means go back further; "none of them holds this database"
  means the backups are fine and this database was not in them — which is
  exactly the shape the `pg_hba.conf` fault above produced for five days
  on a real node.

**And the bug this uncovered.** `fetch` from a bucket took the *last
component* of each key, so every `volumes/<container>/base.tar.gz` in a
backup landed as one `base.tar.gz` at the top of the staging directory,
each overwriting the last, and the restore then found no `volumes/` at
all: **a node restored from a bucket came back with its database, its
manifest and none of its data**, reporting success the whole way. Nothing
failed anywhere — the flat files are the ones a node restore reads first.
`rsync -a` over SSH had always kept the tree; only the bucket path was
flat. The key's path below the prefix is what it lands under now, and a
key with `..` in it is refused rather than written — a download must not
be a way to write anywhere on the disk.

**What is still not there**: `restore --from` takes a local path only, so
naming *one specific* remote backup for one database is not possible —
the search picks the newest that can reach the moment. `restore-node
--from s3://…` names one and now fetches it correctly.
