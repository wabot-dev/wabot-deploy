# The road to production

This is the plan for making wabot-deploy something an operator can put
real traffic on, and something an outsider can evaluate. Like the other
plans here it is written to be argued with: each item says what it is,
**why it comes where it does**, and the trap that is already known.

Two rules carried over from the rest of these documents. What has not
run on a node is not done — almost every hard bug in this project was
invisible locally. And a plan that is only a list of features is a list
of things somebody wanted, not a road: the ordering is the content.

## The premise that orders the list

**The multi-node story today adds failure surface without adding
availability.**

A service can be placed on three machines. The edge picks upstreams by
turn, so a node running two copies gets twice the traffic — that is the
whole of the load balancing and it works, measured at 1.89× on two
nodes. What it does not do is *ask whether an upstream answers*. A dead
replica keeps its share of the traffic until the owner notices, and
noticing is a person looking at a page.

Databases are the same shape one level down. A standby that stops
following is not detected; nothing promotes; `database.primary_slot`
exists precisely so that promoting is a row rather than a migration, and
nothing writes it. So the honest answer to "what happens when a node
dies" is: some fraction of requests fail, silently, until somebody looks.

Everything else on this list is worth doing. None of it is worth doing
*first*, because a scheduler that places replicas across machines with
no health signal is spreading risk rather than reducing it — it places
well once and never reacts.

---

## 1. A licence, and a security posture

Not a feature, and first anyway: **without a `LICENSE` file nobody can
legally use this, or evaluate it.**

Adding the file is five minutes; choosing what goes in it is not, and
this document said otherwise until somebody read it. For infrastructure
that might one day be sold or offered as a service, the choice decides
what a competitor may do with it and what a customer's lawyer will say
about it — permissive (Apache-2.0, MIT), or reciprocal against hosting
(AGPL). It is a strategic decision that happens to produce a small file,
and treating it as a small file is how it gets made badly.

Everything below is blocked on it in the sense that nobody outside can
evaluate any of it. Nothing below is blocked on it in the sense of
work: the code carries on.

`SECURITY.md` matters nearly as much, and for a specific reason this
repository has earned: the argument for holding credentials in the clear
is already written and it is a good one —

> encrypting them against a key kept beside them is a ritual rather than
> a defence

— but it lives in a comment inside `migrations/0030_database.sql`, where
the person doing a security review will never find it. A posture that is
undocumented reads as a posture nobody took. Say it in the open, next to
the file permissions on the data directory and the one-time reveal the
console already uses for secrets, and it stops being a finding.

Then: `CONTRIBUTING.md`, a `CHANGELOG` that is not `git log`, and a
compatibility statement — which versions migrate to which, and what a
node does when it meets a schema newer than itself.

## 2. Health, and failover across the upstreams of one name

The gap in the premise above. Three parts, in this order:

- **A health signal per upstream.** Not "is the container running" —
  reconciliation already asks that. Whether the thing behind the port
  answers. Cheapest honest version: a TCP connect on the upstream's own
  address and port, on the node that routes.
- **Out of rotation, and back in.** The edge picks by turn from a list;
  the list becomes "the ones that answered recently". A node that comes
  back rejoins without anybody pressing anything.
- **Say it.** A replica that is out of rotation must read as out of
  rotation on the page, with the reason. This is where the alert surface
  below gets its first real input.

For databases the same question has a different answer, and it needs a
Postgres client in the binary to ask it: `pg_stat_replication` on the
primary, or `pg_last_wal_replay_lsn` on the standby. That dependency has
been deferred twice and this is where it gets argued for. **Promotion
stays manual** — a node that promotes on its own is a node that produces
two primaries the moment a network partition looks like a death.

## 3. One page that says what needs you — done

The node already knows what is wrong and keeps it in five places:
`doctor`'s problem count, errands that failed, certificate failures on
`node_state`, `replica.last_error`, and the orphan directories on disk.
An operator has to visit five pages to assemble a picture.

This is aggregation, not instrumentation — which is what makes it the
best effort-to-value on the list. It is also the first thing an
evaluator sees, and "the console told me before I asked" is the
difference between a demo and a product.

Notifications by mail or webhook come after, and only once the set of
things worth being woken for is settled by having looked at the page for
a while.

**Built as `console::attention`**, on the page an operator lands on, and
**absent when there is nothing** — the property that makes the rest
worth anything, because a panel that is always there is wallpaper by the
second week. Five sources: a copy that will not start, copies out of the
rotation, a certificate that would not issue, an instruction another
node refused, and storage nothing claims. Every entry carries where to
go, because a list of complaints with nowhere to go is one people learn
to scroll past.

Writing it turned up a third instance of the same fault in one day: the
translation scan enumerates the console's modules by name, so a new
module is invisible to it. `attention.rs` was written, wired into a
page, and left out — every string on the new card unguarded, suite
green. `the_scan_reads_every_module_of_the_console` reads the directory
and compares. **A guard that enumerates its inputs is right until
somebody adds an input**, and that is now three for three: the badge
words covering one of two functions, the settings-redirect list naming
three of six forms, and this.

## 4. What a node has, and what a container may take — done

The inputs a scheduler needs, and useful before there is one.

**Limits.** `memory.max` is written and `/dev/shm` follows it. CPU is
not: nothing writes `cpu.max`, so a container may take every core.
Adding it is small — the same place in the spec, and CPU is already
measured in millicores, which is the unit that makes a limit and a
reading comparable.

**Requests, as distinct from limits — decided against.** A limit is what
a container may not exceed; a request is what a scheduler must reserve,
and Kubernetes separates them to buy density: the sum of requests fits
while the sum of limits does not. What that costs is that the
interesting failures happen under load, on the fullest machine, at the
moment somebody was already busy.

**Here the limit is the reservation.** A node with 1 GB holds four
256 MB services and refuses the fifth. Conservative, and predictable —
and for a platform whose whole claim is "one binary on a box you own"
the second is worth more. Overcommit, if it is ever wanted, is a second
column and a page explaining the trade, not a default discovered during
an incident.

**Capacity, allocatable, committed.** The node measures what it *is
using* — `node::memory`, `node::cpu`, `node::disk`. A scheduler needs
what is *available to promise*: total, minus what the node reserves for
itself and the OS, minus the sum of requests already placed. That
reserve should be configurable and should have a sane default, because
a node that promises its last 200 MB is a node whose own console dies
first.

**Built:** `cpu.max` from a millicore ladder, memory offered for plain
services too — it was a database's alone, so a container could take the
machine's memory with nothing on any page to stop it — a node reserve
that is a fraction *and* a floor (15 % of the 1 GB test node is under
what this process and containerd use; a flat 256 MB on a 32 GB machine
is a rounding error), and a refusal that names the numbers.

Two things worth carrying. The judgement is **pure** and separate from
the four readings it needs: the first version was not, and it refused
*everything* on a machine whose total it could not read — a rule made
of a missing number. What cannot be measured is not enforced. And a
service's own ceiling is subtracted before the sum, or raising one is
refused by its own current value: a form somebody can set once and never
change their mind in, which is worse than no check because it looks like
a rule.

**Disk quota is not like the other two, and should be planned
separately.** Volumes share a filesystem and nothing enforces a share of
it. Doing it properly is XFS project quotas or a loopback filesystem per
volume — a different order of work from writing one more field into the
OCI spec. Measuring disk per replica already works; enforcing it does
not, and pretending they are the same size of job is how a sprint
disappears.

## 5. Backup, and a restore that produces the same node

**"Backup" is three different things here** and only one of them is
hard:

- **The node's own database.** Easy, and already done on every update:
  `VACUUM INTO` a copy, because a byte copy of a live SQLite is a copy
  of a half-finished transaction.
- **Images.** ~~Do not back these up. They are in a registry and come
  back with a pull.~~ **Wrong, and the question that found it was
  "would the restored node run?"** A public base image does come back
  with a pull. A node's *own* build does not: this node's registry is
  the only copy, and it is on the disk that just died. So what is kept
  is what nothing else holds, deduplicated by digest across the whole
  network — see below.
- **Volumes.** The hard one. A file copy of a running Postgres data
  directory is a torn copy that restores into a database that will not
  start — the same class of mistake as copying SQLite by hand. A managed
  engine needs its own tool (`pg_basebackup`) or a filesystem snapshot;
  a plain volume needs the container quiesced or the copy accepted as
  crash-consistent, which is a decision the operator should be asked to
  make rather than one this node makes silently.

**And restoring a node is restoring its identity.** The id is minted at
`install`, kept for ever, and is what every other node calls this one.
Around it sit the WireGuard private key, the enrolment secrets, the
capability grants, and the certificates. A node restored with a fresh id
is a *different node* to the network: it has to re-join, and every other
node goes on holding rows about a machine that no longer exists.

So a restore has to ask a question, out loud: **am I the same node, or a
new one?** "The same" restores the identity with the data and the
network never notices. "A new one" restores the data alone and takes a
re-join. Both are legitimate; guessing is not.

### What was built, and what it took to find

`backup`, `restore` and `restore-node` are in the tree, with point-in-
time recovery on top of WAL archiving. **Verified on the nodes**: PITR
end to end (three rows deleted and recovered into a second database,
the original untouched), image deduplication (ten blobs written, then
zero on the second run), and a full node restore on Alpine — a planted
ghost project gone, the identity kept, and the **WireGuard session never
interrupted**, which the handshake age and byte counters proved
afterwards.

Four things that only a node run could have said:

- **One root for the whole network, and the name is the hash.** An OCI
  blob is addressed by the sha256 of its contents, so a file already
  there under a digest *is* the blob for that digest. Skipping what
  exists is not an optimisation, it is the definition — and it makes a
  shared root safe to write from several machines at once, because the
  worst two simultaneous writers can do is write identical bytes.
- **A backup carries what something claims.** A volume directory
  outlives the copy that made it, deliberately. So the disk holds
  directories for copies moved off or thrown off this node, and copying
  them is weight in every backup for ever with no row to restore them
  under: 62 MB of a moved database on Alpine, which was *the entire
  backup* — 62.4 MB became 496 KB. Skipped and **named**, because "my
  backup has everything on the disk" is the assumption, and a backup is
  the worst place to be quietly wrong.
- **Unknown is not empty.** The dangerous reading of an unreadable
  query is "nothing is claimed", which turns one failure into a backup
  that holds no volumes and looks like a backup. With no list,
  everything is claimed — the shape `fits` already takes for a machine
  that cannot say how much memory it has.
- **`archive_timeout` is a floor, not a promise.** Measured two to three
  minutes for a value of sixty seconds. The recovery window is wider
  than the setting says.

### The restore report, and the three faults it uncovered

A restore now ends by naming what has to reach this machine, where each
name points, and what address this machine goes out from — **shown, not
judged**. `backup` is deliberately left alone: a node whose DNS is
broken needs a backup *more*, DNS is not this node's state and will have
changed by restore time, and tying a local offline operation to a
network lookup is a backup you do not have when a resolver is slow.

Writing that one report turned up three separate faults, all invisible
locally, all behind a single `database is locked`:

- **A SQLite database is three files.** Replacing `node.db` with
  `std::fs::copy` left the *previous* database's `-wal` and `-shm`
  beside it, so the next open replays one database's pages into another
  — and produces a file that opens.
- **`close` is a checkpoint, not a close.** It takes `&self`; the writer
  connection lives inside the handle until the handle is dropped.
- **A stop returns before the process does.** `rc-service stop` printed
  `[ ok ]`, `status` said `stopped`, and the node drained for **seven
  more seconds**. The guard asked `is_active` — the ledger answering
  about history — and now asks the process table, which is the thing.
  Same rule as install steps, in a place nobody expected it.

That third one had a consequence worse than the lock: `--new-node` calls
`forget_self` *after* the copy, so restoring on a running node gave the
operator the old identity, an error, and no sentence telling them to
re-join.

### A restore onto a new machine with a different address

Reasoned from the code and **not yet run**, which in this project is the
difference between an answer and a claim. Two things decide it, and both
are already true:

- **A node's advertised endpoint is a name, not an address** —
  `{domain}:{port}`. What a node says about itself carries no IP.
- **The other direction is learned from the handshake.** A node that can
  dial sets a keepalive; the peer's address is whatever its packets came
  from. That is WireGuard roaming, and `network::tunnel` was built
  around it.

So a **joined** node restored onto a new address should heal with no
manual step: it dials its authority by name, and the authority learns
where it now is. A **public** node needs its DNS repointed first,
because everything that reaches it reaches it by name — the console,
other nodes dialling in, and the ACME challenge. Until then `acme::ensure`
**refuses to order** rather than burning validations against an
authority that locks the account at five an hour: a check built for a
moved hostname, covering a moved machine for free.

Worth running before anybody depends on it.

## 6. Placement that decides

Only now. A scheduler is a function of the signals above — capacity to
place against, health to react to, requests to reserve — and it is worth
saying what it must *not* be:

- **Start deliberately dumb.** Spread replicas of one service across
  distinct nodes, respect the `host` grant, respect requests. That is
  most of the value and it can be explained in a sentence, which is what
  makes an operator trust it.
- **A placement is still a row.** Whatever the scheduler decides, the
  result is the same `replica` rows a person could have written by hand,
  and a person can still override them. A scheduler whose decisions
  cannot be read or overruled is one nobody will run in production.
- **Draining before cleverness.** "Take this node out of service, move
  what it holds" is worth more than any bin-packing heuristic, and it is
  the operation every upgrade and every hardware failure needs.

## 7. Disk that cleans up after itself

Four kinds of rubbish, all known and none collected:

- **Images.** Nothing garbage-collects them. 1.6 GB of containerd on a
  20 GB test node, which the new disk card now makes visible.
- **Orphan directories.** `doctor` reports all four kinds now — data,
  config, hosts, logs — and removes none. Reporting was the right first
  step; a `doctor --clean`, or a button, is the second.
- **Database copies from updates.** One per update, ten on the Ubuntu
  node, nothing has ever removed one.
- **Container logs.** Bounded now at 8 MB each. The bound was the fix;
  a retention policy is still a thing this does not have.

Each of these is small alone. Together they are the difference between a
node that runs for a year and a node that fills up in six months.

## 8. What an enterprise asks for on the second call

- **An audit log.** Who did what, when. There are roles — administrator,
  owner, deployer, viewer — and no record of what anybody did with them.
  Cheap to add, and asked for in every procurement.
- **Metrics somebody else can scrape.** CPU, memory and disk are already
  measured per replica and per node. A Prometheus endpoint exposes what
  exists. Enterprises do not watch your console; they scrape it into the
  one they already have.
- **Rate limiting and lockout on the console.** There is password
  authentication and no backoff.
- **A signed release.** The checksum beside the binary is good. Signing
  it is what lets somebody verify it came from you rather than from
  whoever served the page.

---

## The order, and why it is this one

| | | why here |
|---|---|---|
| 1 | Licence, security posture | nothing else can be evaluated without it |
| 2 | Health and failover | the multi-node feature is not one until this exists |
| 3 | The alert page | aggregates what is already known; first real input from 2 |
| 4 | Limits, requests, node capacity | the scheduler's inputs, useful alone |
| 5 | Backup and restore | data safety before more machines |
| 6 | Scheduler | needs 4 to decide and 2 to react |
| 7 | Disk cleanup | a year of uptime rather than six months |
| 8 | Audit, metrics, signing | the second conversation, not the first |

## What is deliberately not here

- **Automatic promotion of a database primary.** See §2. A node that
  decides on its own that a machine it cannot reach is dead will
  eventually decide it during a partition, and two primaries is worse
  than an outage somebody was told about.
- **A rollback button for updates.** The two halves exist — the previous
  binary and the pre-migration database copy — and `doctor` names them
  now. What is missing is not a button: rolling back a schema is not a
  file operation, and pretending one button does both is how somebody
  loses the data the copy was taken to protect.
- **Encryption at rest for secrets, against a local key.** The argument
  is in §1 and it stands. What changes this is a key that lives
  somewhere else — an operator's KMS, or a passphrase entered at boot —
  and that is a feature with a cost worth discussing, not a checkbox.
- **More engines.** Postgres first was chosen so that the second is a
  table of numbers and two strings. That is still true and it is still
  not urgent: nobody has asked.
