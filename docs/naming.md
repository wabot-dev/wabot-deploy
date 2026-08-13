# Names, certificates and reaching things

A service should reach a database by name, over a connection it can
verify, whether the database is on this node or another one. That is one
sentence and four pieces of work.

## The decision this rests on

**A database's name is a name like any other, and its certificate is
chosen the same way.**

`edge::policy` already asks, per name, what to do when a certificate is
needed, and the three answers are the three anybody wants:

| | What it means |
|---|---|
| `Acme` | Ask a public authority. The default wherever ACME can work. |
| `SelfSigned` | Sign it here, with the node's own CA. For a name no public authority can validate. |
| `File` | A key somebody else keeps fresh. The node reinstalls what it finds and does not pretend it can renew what it did not issue. |

An earlier draft of this document proposed building an internal CA for
database names as a phase of its own. That was the same thing under a
different name: `SelfSigned` **is** the node's CA, and it has been there
since before this was written. A database inherits the choice by being a
name, and there is nothing to build for it.

What was missing was noticing that a database can be **published through
the same edge**: `acme::wanted_names` asks for a certificate for every
name this node was chosen to serve, and `certs.rs` picks one by SNI. A
database's name goes through that path unchanged, whichever source is
behind it.

So, with `Acme` — the default:

- **From outside**, the name resolves to the node's public address, and
  the connection is TLS with a Let's Encrypt certificate. A client
  verifies it with the trust store it already has. Nothing to
  distribute.
- **From inside**, the same name resolves to an address on the project's
  bridge. Same name, same certificate, same `verify-full`.

Split-horizon resolution is usually painful because it needs two
certificates for one name. Here there is one, and the name is the same
in the connection string a developer writes on their laptop and the one
their application uses in production. That is the property worth having.

**And with `SelfSigned`, the same picture holds inside and simply stops
at the edge of the node's network.** A public name is a row in DNS and a
line in a Certificate Transparency log, which announces that this
database exists — a real reason to choose the other source. The one
thing it adds is that a client has to trust the node's CA, so the node
mounts `local-ca.crt` into every container it starts. An application
then verifies with `sslrootcert=/etc/wabot/ca.crt` and nothing in the
image has to know anything.

`File` needs the same mount only if whatever signed the key is not
already public.

## TLS lives in the server, and the first attempt had it in front

The first version of this put a TLS terminator on the node in front of
Postgres — which meant learning that Postgres does not begin with a
handshake at all. A client opens a plain socket, sends eight bytes (a
length of 8 and the code `80877103`) and waits for a single byte back,
`S` or `N`; only then does TLS start. A plain listener sends a
ServerHello where the client is waiting for one byte.

That was all true and it was the wrong shape, for a reason a question
found rather than a test: **a terminator covers a published port and
nothing else.** A container on the project's own bridge reaches the
database's container directly, and no proxy can stand between them. So
the most common path — an application talking to its own database —
would have stayed unencrypted, and `pg_hba.conf` would have had to keep
saying `host` where it should say `hostssl`.

The terminator was deleted. TLS is in the server, with the certificate
the node places, and **not one line of `pg_hba.conf` would accept an
unencrypted connection** — there is a test that walks the file and fails
if anybody writes `host` where `hostssl` belongs. The only exception is
the unix socket, which lives inside the container and without which the
image's entrypoint cannot finish `initdb`.

Verified on a node: `sslmode=disable` is refused with `no pg_hba.conf
entry … no encryption`, `sslmode=require` gets TLSv1.3, and
`sslmode=verify-full` against the node's CA passes by name.

## Two names, because one would break every write

A read replica answers reads and refuses writes — on a node,
`cannot execute INSERT in a read-only transaction`. So a single name
round-robining over a primary and its standbys fails a share of every
application's writes, in proportion to how many replicas there are.

- `orders.<project>.<domain>` — the **primary**. Writes and reads.
- `orders-ro.<project>.<domain>` — the **read pool**. Every standby,
  and the primary only when there are none.

The pool resolves to every standby, and **the order is the reader's**:
each container is given the addresses rotated by a hash of its own name,
so ten applications in a project do not all put the same replica first
while the others hold data nobody looks at.

That is spread, not balance, and the difference is worth stating: one
reader still sends every connection to one replica until it stops
answering. Per-connection balancing is `load_balance_hosts=random`,
which libpq has done since PostgreSQL 16 and which shuffles exactly
these addresses — the node stays out of the path and the TLS session
runs end to end to whichever replica the client chose.

A proxy of the node's own, holding an address on the project's bridge,
would balance per connection. It would also put the node in the path of
every query, need an address managed on an interface CNI owns, and know
too little about Postgres to health check what it was balancing. That is
a phase with its own justification rather than a detail of this one.

**The certificate covers both names.** A client dialling `orders-ro`
with `verify-full` checks the name it asked for, so a certificate
holding only the primary's name would fail every read — the pool is the
same database, so it is one certificate with six names on it rather than
two certificates.

## Reaching a replica on another node

**The edge already does this.** Phase 7 teaches a node to answer for a
name it does not own: it claims the name, obtains its own certificate
for it, and proxies to the upstreams it was given. A read replica
elsewhere is the same shape with a different terminator on the end.

So cross-node reads need no new distribution mechanism, no shared CA and
no certificate copying. The node holding the replica gets its own
certificate for the pool name, through ACME, exactly as it does for a
web service today.

## Resolving the name inside a container

A container has no way to resolve any of this today: the node
bind-mounts a `resolv.conf` naming the host's public resolvers, and
nothing else.

Two mechanisms, and the first is enough for a long time:

**A hosts file per container**, written from the rows and bind-mounted.
It is rewritten *in place* whenever anything changes — and because it is
a bind mount, rewriting it on the node changes what a running container
sees immediately. `getaddrinfo` reads the file on every call, so a
database added this morning is reachable from an application started
last week without redeploying it.

Measured on a node: inside the running primary, `getent hosts orders`
answers `10.42.2.200` and `orders-ro` answers the standby; a name
appended to the file on the node was resolvable inside a container that
never restarted.

And the hazard the design turns on was measured too. `sed -i` renames
rather than edits, so it replaced the inode: the node held 792415 while
the container went on reading 792419, and a deleted name stayed
resolvable until the container was recreated. `std::fs::write` truncates
the same inode, which is why nothing in `deploy::hosts` may ever be
"improved" into a write-to-temp-and-rename — and why editing these files
by hand detaches them.

Two things it cannot do, and they are why the second exists: no
wildcards, and no round-robin across several addresses for one name.
Neither matters while the read pool is balanced by the terminator rather
than by the resolver.

**A resolver on the node**, later, if wildcards or DNS-level balancing
turn out to be wanted. It reads the same rows and serves the same names,
so nothing above it changes. It costs a listener on port 53 bound to
each bridge's gateway — an address that does not exist until CNI creates
it, which needs `IP_FREEBIND` — and an open resolver is an amplification
vector if the binding is ever got wrong.

Names are **scoped to the project**. A container is given its own
project's names and no others, because two projects' bridges are
separate L2 domains and `runtime::network` says plainly that the
isolation is the point of separating them. A name that crossed it would
be a hole opened by a naming convention. Reaching another project's
database is a thing somebody should have to ask for, with a row saying
so, and it is not in these phases.

## What has to be true for the certificate to arrive

The name must resolve to the node before it is accepted — `ports::create`
already checks this, and the reason is in its comment: a route for a name
that points somewhere else is a certificate request that fails, twice a
day, against an authority that locks the account after five failures.

A database's name is therefore not free-form. It is registered the way a
service's is, and the same check refuses it.

## The ownership problem, and how it was answered

Postgres refuses to start if its private key is readable by anyone but
its own user, and every file the node writes belongs to root. The user
is 70 on the alpine variant of the image and 999 on the debian one, so a
node that hard-coded either would be right until somebody changed the
tag.

Nothing guesses. The key goes in the volume — beside `pgdata`, not
inside it, because `initdb` refuses a data directory that is not empty —
and the **image is asked**: a one-shot container runs
`chown postgres:postgres`, once, and after that the directory itself
records the answer. Every later write reads the owner back off the disk
and restores it.

That last part is not a nicety. Writing a file makes it root's again, so
without it a renewal would hand the server a key it cannot read — the
same failure as the first placement, arriving three months later when
nobody is looking.

## Rotation

A renewed certificate reaches a running database over `SIGHUP`, which
makes Postgres re-read its TLS files. Ninety-day certificates therefore
do not cost an outage every ninety days.

The pass is convergent — it asks whether the file matches the store, not
whether anything renewed — and it runs once at start as well as on a
timer, because a node that was down while a certificate was renewed
should hand it over when it comes back rather than a quarter of an hour
later.

It also **forgets**. The store used to be one-way: once a name had a
certificate it kept it for ever, so renaming a node, clearing its
domain, dropping an edge or a database learning it belongs to somebody
else each left a row behind — reissued twice a day for a name the node
does not answer for, and listed by `doctor` among the ones it does. Found
as two certificates for one database, one under each node's domain, and
the same pass turned up five more on the other node whose services had
been deleted months earlier.

Two rules stop it being the mistake it could be. **A `File` source is
never pruned**: a self-signed certificate this node can make again and an
ACME one it can order again, but a file somebody put here it can only
lose, and a convergent pass must not be able to destroy something
unrecoverable. And **the wanted set has to be complete or the pass does
not run** — three unrelated sources feed it (the edge's names, the
resolver's fallback, the databases, whose names were never edge names at
all, which is how one came to be stored twice), so a source that cannot
be read means a short list, and a short list here deletes certificates
that were working. An empty list deletes nothing: no node legitimately
wants none, so that is a bug upstream rather than an instruction.

It also **reissues**, and finding out why is the reason it does: a
self-signed certificate for a database expired with nothing to renew it,
because a database's internal name is not a name the edge was asked to
serve, so ACME never looks at it. What the pass reissues is decided by
where the stored certificate *came from* — `StoredCert::source`, which
exists because the renewal loop used to read `issuer` as a decision and
silently replaced anything it did not recognise. ACME's and a file's
belong to whoever keeps them fresh; the node's own are the node's to
replace.

## A name belongs to the database, not to the machine holding a copy

The long name was built from **this node's** domain, for everything on
this node. That is right for a service this node owns and wrong for a
copy it is merely holding: the same database then answers to
`orders-ro.db-test.<alpine>` on one machine and
`orders-ro.db-test.<ubuntu>` on another, each with a certificate
matching only its own, so no single connection string reaches it. A
qualified name that changes with where you read it is not a qualified
name.

Measured on the Alpine node, connecting to its copy by the owner's name:

```text
psql: server certificate for "orders" (and 5 other names) does not match
host name "orders-ro.db-test.wabot-deploy-testing.dev.tobaw.shop"
```

So the owner's domain travels with the errand and is kept on the row
(`database.owner_domain`, migration `0032`). Both places that build a
name read it: `hosts::entries_for` takes the suffix **per service**
rather than one for the whole file, and `certificate_names` prefers the
row's over the node's. A service of this node's own has no row and falls
back to this node's domain, which is the same answer as before.

The interesting part is that the fix was correct and reached nobody,
twice over. `adopt` read the domain off the errand into its parameter
tuple and then left the column out of the `INSERT` — a bound value
nobody uses is not a warning, and no test looked. And the errand itself
was only ever recomputed by a deployment, so a payload that gained a
field went on being the payload from before the upgrade. Both are the
same lesson as phase 7's: **derived state needs something that
recomputes it, and boot is when.** Reconciliation dispatches now, and
`queue_if_changed` makes it free when nothing moved.

## Phases

| | Phase | Delivers |
|---|---|---|
| 1 | Names inside | A hosts file per container, rewritten live from the rows; a database and a service reachable by name within their project — **done, verified on a node** |
| 2 | TLS in the server | `hostssl` on every line, the certificate placed and rotated by the node — **done, verified on a node** |
| 3 | The read pool | The second name, spread across the standbys per reader, and covered by the same certificate — **done, verified on a node** |
| 4 | Across nodes | The pool name served from the node holding the replica, over the edge errand that already exists |
| 5 | Client certificates | `clientcert=verify-full` in `pg_hba.conf`, and a service authenticating with a certificate instead of a password |

Choosing the source is not a phase. It is `edge::policy`, which exists,
and the only thing phase 2 adds for it is mounting `local-ca.crt` where
a container can find it.

Phases 1 to 3 are one node and are worth having on their own.

**A copy on another node is named and verifiable** — that came with
phase 4 of `docs/databases.md`, and it is what "the ownership problem"
above is about. `sslmode=verify-full` against the owner's qualified name
succeeds on both nodes (2026-08-13): writable where the primary is,
read-only where the copy is.

What phase 4 here still owes is **reading a pool from a node that holds
none of it**. A name resolves inside the project that holds the copy and
nowhere else, so an application on a third node has no address for
`orders-ro`. That wants a forwarder with an address on that node's
bridge, which is the edge's shape applied to a TCP port rather than a
hostname.

Phase 5 is where "what an enterprise expects" actually lands: a password
in an environment variable is the thing certificates exist to replace,
and `pg_hba.conf` can require a client certificate per user. It is last
because it is worth nothing until the rest is true.
