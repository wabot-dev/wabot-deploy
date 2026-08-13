# A network of nodes

Public nodes are entry points; private nodes run containers. Any public
node can be an edge for a name whose container lives on a private one,
and a name can eventually be served by a group of them.

## What it is for

**A service is administered from the node where it was created**, and
never by going to the machine it happens to run on. That node is where
somebody says how many replicas it has and where each of them goes —
here, or on a node that joined. Staying local is a placement like any
other.

What lands on the receiving node is **derived**, and reads that way: the
project and the replica are not editable there, because the instruction
came from somewhere else. The one thing that node's operator can always
do is **evict** it — a danger zone that says where it came from and
removes it. Not modify: modifying would be two nodes disagreeing about
one service with no way to settle it.

That is the same rule the grant already follows in the other direction.
The machine is yours even when the orders are not, and joining is not a
loss of control.

**The owner also decides who serves it.** A service's edges are chosen
from the same page, and they can be *any* public node on the network —
not only the one the service lives on. So the owning node configures two
things about a service: where its replicas run, and which public nodes
answer for its name.

That is what the `claim` table from phase 0 was written for. Any node
can ask any public node to serve a name, so the second claim on one
hostname is refused and names who holds it — a machine cannot resolve
two authorities pointing one name at different backends, and choosing
silently would make the wrong one look right.

Three consequences that shape everything below:

- **A replica is the unit**, not a service. A service has a number of
  them and each names its node, repeats allowed — two on one machine is
  a thing somebody will want. So a container id has to carry the replica
  index rather than being derived from the service, which reaches the
  runtime, the reconciliation on boot, and routing.
- **State has to travel back.** A page that administers a replica it
  cannot see the state of is a page that is lying. The poll a node
  already makes is where that goes.
- **An edge reaches a replica through its node, not at its container.**
  A container's bridge address is `10.42.<project>.<n>`, which is not
  unique across nodes — two nodes each numbering a project 2 is normal
  and has to stay normal. And the overlay is a `/24` of *node*
  addresses on purpose. So a replica that something elsewhere has to
  reach needs a port published on its node's overlay address, and the
  edge proxies to `10.42.0.<node>:<port>`. `proxy.rs` still needs no
  changes — it already proxies to an arbitrary address — which was the
  whole reason for choosing an overlay in the first place.

  It also keeps the edge dumb: the owner knows where its replicas are
  and whether they are up, so the errand carries the upstreams. An edge
  that had to discover them would be a second thing with an opinion
  about where a service is.

This document is the plan and the record of what was decided and why.
Phases 0 to 7 are in the tree and verified between two real nodes.

**What 5 and 6 left open is closed.** A replica placed on another node
and then brought home used to leave its container running there with
nothing naming it — the page that placed it had no way to say stop. Both
errand kinds now carry the *whole* of what a node should be running: a
`host` errand names every slot that node holds, and an `edge` errand
every upstream for that name. **An empty list is a real instruction**,
not a reason to skip sending one, which is how a node finds out that its
last replica went home or that it no longer serves a name.

## The shape

**Authority is directed. There is no agreement to reach.**

The obvious design for "several nodes share a configuration" is
consensus, and it is an order of magnitude more machinery than this
product should carry. It is also the wrong question. A node does not
need to agree with its peers about the world — it needs to know **which
of them it takes instructions from**.

So a node *grants authority* to another, and the holder sends *errands*:
"host this container", "be the edge for this name". Two nodes that
granted each other nothing cannot affect each other, and nothing has to
be reconciled because nothing is shared.

Three consequences worth stating, because they are what makes this
cheap:

- **Joining is not a loss of control.** It is one row, written
  deliberately, and revocable from the node that wrote it.
- **The shared-configuration phase mostly disappears.** It becomes
  "instructions and their acknowledgement" rather than replicated state.
- **Certificates mostly take care of themselves.** An edge is public, so
  it can answer ACME for the names it serves. Sharing only becomes
  necessary when two edges serve one name and DNS spreads the challenge
  between them — a deferred problem, not a blocking one.

**Public and private is about reachability, not policy.** A public node
has an address the internet can dial. That is the entire difference, and
it is why `Kind` is derived from having an endpoint rather than trusted
from a setting somebody can set wrongly. An unrecognised kind reads as
private: a node we cannot place must never be offered as somewhere to
send the internet.

**A name belongs to one authority.** A second claim is refused rather
than merged or overwritten, and the refusal names who holds it. Two
authorities pointing one hostname at different backends is not a
conflict a machine can resolve, and choosing silently would make the
wrong backend look like the right one. Claiming a name you already hold
succeeds — an errand sent twice must not fail the second time.

## The overlay

WireGuard, not a reverse tunnel. The deciding reason is that `proxy.rs`
needs **no changes**: it already proxies to an arbitrary address, and
across an overlay a private node is simply `10.x.y.z:port`.

It works behind NAT because the private node opens the UDP session
outbound and `PersistentKeepalive` holds the translation open, so the
public node can send packets back without ever having dialled. The
condition is that the public node has a stable address — which is what
makes it public.

**Kernel, and the spike is why.** This section used to argue for
`boringtun`: kernel WireGuard is faster, but a one-core node running TLS
and HTTP runs out of CPU somewhere else first, and requiring a kernel
module reopens the class of failure Alpine keeps surfacing — no
iptables, no cgroups mounted, no overlay module. A userspace crate keeps
the one-binary promise.

The spike, run on both nodes, said the premise was wrong:

- The `wireguard` module is **in both kernels** (Ubuntu 26.04 / 7.0,
  Alpine 3.23 / 6.18), and `ip link add type wireguard` creates the
  interface with **no `wireguard-tools` installed**.
- On Alpine, **`/dev/net/tun` does not exist** — it appears only after
  `modprobe tun`. Userspace WireGuard needs it.

So userspace does not avoid a kernel module. It swaps `wireguard` for
`tun`, adds a device node that is not there, and carries the whole data
path in this process. Every argument for it was an argument against a
cost the kernel path turned out not to have.

`wg-quick` was never on the table anyway: it is a bash script, and
Alpine has no bash.

**Netlink from inside the binary, not `wireguard-tools`.** Neither node
had `wg` installed, and installing it would put a package manager in the
path of joining — on Alpine especially. `defguard_wireguard_rs` speaks
the kernel's generic-netlink API directly: no C, no packages, and its
`Key` is the same Curve25519 key `network::keys` stores, so the binary
carries one copy of curve25519 instead of the two a separate x25519
dependency cost.

## Enrolment

From a public node: "add private node" mints a token. The token carries
the public node's id and name, the endpoint its control plane answers
on, its overlay public key, its own overlay address, the address it
allocated for the joiner, and a bearer secret.

Spending it has **two doors and one implementation**. `wabot-deploy join
<token>` is the sibling of `install`, for a machine somebody is already
logged into; the joining node's own console has a form, for the far more
common case of a node that is already installed and already answering,
where sending somebody to find its terminal would be sending them away
from where they are. The order of the writes below is a safety property
rather than a detail of how it was typed, so it lives in
`network::join` and both doors are thin.

The pattern already existed twice — `setup-token` and people invitations
are both "a single-use secret, shown once" — and the push-token fix from
the same work was the template for showing it without putting it in a
URL. That is what this uses: the redirect carries a nonce, the page
spends it, and what is stored is the hash.

Written, and these are the decisions inside it.

**The token is one opaque blob, versioned.** `wdj1.` then unpadded
url-safe base64 of the fields. Six flags would be four chances to
transcribe something wrong, and three of the four failures do not
surface until a tunnel does not come up. The prefix means a paste is
recognisably a join token, and a later field can be added without an
older node reading a newer token as a corrupt one. Single use, 24 hours,
shown once, hashed at rest.

**The joining node brings its own identity; the authority allocates only
an address.** Tempting to allocate the id here too — the address is
allocated here — and wrong: a node that joins two authorities would be
two nodes with two identities, and "which node ran this" would have two
answers. So the id is minted at `install` and travels in the callback.
The cost is that a claimed id could collide with one already known, so a
collision is *refused* rather than merged; the upsert underneath would
otherwise let a joining node overwrite the row it collided with,
including the authority's own.

**The callback is the one exchange that runs the other way.** Every
other message travels from the authority to the node that granted it.
This one has to go back, because after `join` has written the grant the
authority still does not know the node exists. Ordinary HTTPS on the
control plane — the same hostname and certificate the console is on, no
new listener, nothing to do with the overlay.

**`join` calls before it grants.** Both orders leave something to tidy
up when the exchange half-fails. This one's leftovers are harmless: an
authority that knows about a node not yet obeying it can be re-joined,
whereas a node obeying an authority it never reached would have granted
power over itself on the strength of a pasted string. Both halves are
convergent, so the fix either way is to run it again with the same
token — spending is idempotent *for the node that spent it*, which is
what makes a lost response survivable.

**The authority needs a publicly trusted certificate.** Bundled roots,
as everywhere else in this binary, so a node serving its own
certificate cannot enrol anybody. That is a real limitation and the
honest one: the alternative is a joining node that accepts any
certificate and hands its bearer token to whoever answered. The way out,
if it ever matters, is a fingerprint in the token and a pinning verifier
— not a flag that turns verification off.

**Overlay addresses are `10.42.0.0/24`, lowest free first.** That is
the one `/24` project bridges deliberately leave alone; the rest of
`10.42.0.0/16` has always been theirs, one `/24` per project.

It was written as a `/16`, under a comment explaining why that range sat
safely away from what a VPS or Docker would use — checked against the
outside world and not against this product's own container networking.
It worked by two accidents: project indexes start at 1, so the overlay's
first 254 addresses landed in the reserved slot, and a bridge's `/24`
outranks a `/16` in the routing table. The `/16` would have claimed a
route over every project bridge on the machine and handed out
`10.42.1.x` — inside project 1's subnet — at the 255th node.

A `/24` is also the honest description: **an overlay address names a
node, never a container.** Reaching a container on another node goes
*through* that node, which is what lets two nodes each have a project
numbered 2. Nothing in flight had to move — every address already
allocated was inside the `/24`.

Lowest free rather than next, so an overlay whose nodes come and go
stays dense enough to read. A pending token holds its address — it is
already written into something somebody is carrying — and a node that
**re-joins keeps the address it already has**: moving it would mean
reconfiguring both ends of a working tunnel, and a spent enrolment holds
its address for ever, so the old one was never given back.

**Keys exist before there is a tunnel.** Curve25519, generated on demand
the first time a node enrols or joins, private half in `setting`, public
half in the node row. Not a phase 2 decision in disguise: the kernel
module and `boringtun` read the same base64, so this commits to nothing.
It is here because the key travels *in the token*, and a field added to
the token later means every node that already joined has to join again.

Still open:

- **`curl | sh` is outward-facing.** It is the convention and it is
  convenient; it also defines how this product is distributed. Not
  decided, so the console does not print one: it shows `wabot-deploy
  join <token>` and says the other machine has to be a node already.

## Phases

| | Phase | Delivers | State |
|---|---|---|---|
| 0 | Model | `node`, `authority`, `claim`; the claim rule | **done** |
| 1 | Enrolment | Token, `join`, keys, B recording A as authority | **done** |
| 2 | Overlay | Spike, then a real session. `doctor` proves it | **done** |
| 3 | Errand: host | A queues a deploy on B | **done**, in the wrong place |
| 4 | Replicas | A service is *n* placements; the container id carries the index | **done** |
| 5 | Placement | The form moves to the service; provenance, read-only, eviction | **done** |
| 6 | Reporting | The poll carries each replica's state back to the node that placed it | **done** |
| 7 | Errand: edge | The owner picks public nodes to serve a name; they claim it, get the certificate, and proxy to the replicas | **done** |
| 8 | Consent | What a join requires and offers, per capability, shown before it is spent | next |
| 9 | Groups | Health and failover across the upstreams of one name | |

Phases 4 to 7 are **verified between the two nodes** on v0.6.6 — see
"What the nodes said about 4 to 7" below, and the seven fixes it took.

Phase 3 worked and was in the wrong place — its form lived on the
*node* page, pick a service and send it there, while the goal put it on
the *service* page: how many replicas, and where each goes. Phases 4 and
5 fixed that, and **the old form is now gone**, which took longer than
it should have: it stayed for three phases after the thing that replaced
it.

Leaving it was not harmless. It queued a `host` errand with `slots:
[1]`, and a receiving node reads that list as the whole of what it runs
for a service — so pressing it on a node already holding slots 2 and 3
would have stopped those and left it slot 1, silently. And for a managed
database it sent the wrong *kind* of errand entirely: a plain container,
no volume, no engine row, initialising into a layer thrown away at the
next deployment. It would have looked like it worked.

What used to be phases 4 and 5 — an edge routing a name to a container
elsewhere, then groups — is still two, but both moved and the first one
grew: an edge is now anything the owner picks from the public nodes on
the network, and a name reaches *the replicas of a service* wherever
they are, with one of them being local not a special case.

The spike is done and it reversed the overlay decision above. The
interface comes up from the `node` table at every start and whenever a
node joins or is enrolled, and `doctor` reads the peers back **from the
kernel** rather than from what this process asked for — the question is
whether packets move, and the only thing that knows is the kernel.

**Verified between the two nodes** on v0.3.1. Both claims held: creating
the interface over netlink autoloaded the `wireguard` module with no
`modprobe` anywhere, and the Alpine node's handshake reached UDP 51820
on the public one. ICMP and TCP-with-TLS both cross, at 18 ms.

The asymmetry works as designed. The public node has no endpoint
configured for the private one — its `node` row has none to give — and
WireGuard learned `172.104.24.252:51820` from the handshake that
arrived. Nothing guessed an address the NAT owns.

One thing to know before phase 3: a request to `https://10.42.0.2` over
the overlay answers **404**, because the edge on the far side routes by
hostname and a bare address is not one of its names. That is the edge
working, and it is why an errand that hosts a container will proxy to
the container's port on the overlay rather than through the far node's
edge.

## Two things not to discover late

Both were phase 3's, and both are now decided.

**Images live where they were pushed.** The private node **pulls from
the authority's registry** — one of the three originally listed, and the
one that reuses the direction that works: it already reaches the
authority over a certificate it trusts. The errand carries the image
reference and a credential; the image moves only when something needs
it, and only the layers that are missing.

The pull path had no way to carry a credential — `resolver: Default`,
so an authenticated registry answered 401. containerd's transfer API
offers two ways in: a stream it makes auth callbacks on, and a flat map
of headers. The stream is for interactive and token-exchange flows; a
static `Authorization` is what a registry reading Basic credentials
wants and is the whole of what this needs.

The cost of a static header is that containerd sends it to whatever host
the transfer talks to, **including one it was redirected to**. That is
safe here because the registry being pulled from is another
wabot-deploy node, which serves blobs out of its own content store and
redirects nowhere. It would not be safe against a registry that hands
blobs off to object storage, and the note is in `runtime::images::pull`
so it is read before somebody points it at one.

The credential is stored per **registry host**, not per service: every
service whose image lives on the same node authenticates the same way,
and per-service copies would be one secret duplicated per deployment
with nowhere to rotate it. Same shape as `~/.docker/config.json`, for
the same reason.

**Deploying is local, and stays local.** An errand is an *instruction*,
not a job. The node that collects one writes its own local job for it,
so the queue never stops being per-node and job routing never appears.
`deploy` still talks to *this* node's containerd, on whichever node that
is.

**A fourth, found while wiring it:** the node that collects errands has
to authenticate *itself* to the authority, so it needs the secret it can
present — not a hash it can only compare against. The plan had the
authority delivering, which put the hash on the right side; collecting
inverts it. Migration `0018` adds the clear copy on the collecting side,
and it is null for any node that joined before — those re-join, which is
one paste. `is_authorised` is still unused for the same reason: nothing
arrives from an authority to be authorised.

**And a third, which the phase surfaced rather than the plan:** the
authority cannot reach a private node over TLS. B has a self-signed
certificate for a name, not for `10.42.0.2`. The direction that works is
the one enrolment already proved — B reaches A over a publicly trusted
certificate — so **B collects its errands rather than A delivering
them**. The authority still decides; the model is who gives orders, not
who dials. That also means the overlay is not in the control path at
all: it is a data plane, and its reason for existing is phase 4, where
an edge must reach a container on another node.

**Images live where they were pushed.** The registry is per node, and a
private node needs the image to start the container. Either you push to
each, or the registry replicates, or private nodes pull from a public
one's. That is design, not detail.

**Deploying is local.** `deploy` talks to *this* node's containerd. An
errand to host something has to run its job on the target node, so the
queue stops being purely local and job routing appears.

## Where phase 1 stopped

`migrations/0016_enrolment.sql` and `src/network/` — `keys`, `overlay`,
`enrolment`, `token`, `call`, `api`, `join` — plus `wabot-deploy join`,
the nodes page at both ends of the exchange, and a `network` section in
`doctor`.

The model is real rather than described now: `install` and every `serve`
write the self row, the console lists that table instead of a synthetic
list of one, and `node::all()` is gone. What still carries an
`allow(dead_code)` is only what a later phase consumes, each naming its
phase — `is_authorised` and the claim rule for phases 3 and 4, and the
private key for phase 2.

**Verified between the two nodes**, on v0.3.0, installed at both ends
by the updater — which also exercised that path for the first time,
migration and all, including on OpenRC. The Alpine node joined from its
own console, holds `10.42.0.2`, and lists the Ubuntu one as an
authority; the Ubuntu one lists it back.

Three things only the nodes showed:

- **A row about another node describes the relationship, not the
  machine.** The Alpine node answers to a name and has a real
  certificate, so its own page said *public* — correctly — while the hub
  listed it as *private* at the same moment. Both rows were right and
  the pair was nonsense: the hub does not know what that machine is, it
  knows it has nothing but an overlay address for it. Fixed as wording,
  deliberately not as data. Having the joining node report an endpoint
  would make the two agree and would store, on the hub, an address it
  must never use for an errand — the overlay is the way it reaches that
  node — which is a phase 3 bug waiting to be written.
- **`doctor` contradicted itself.** It printed the config file's domain
  under `configuration` and the stored one under `network`, and those
  differ on any node renamed from the console — which is most of them,
  since the file keeps whatever `install` was first given.
- **This node was the second card**, because the list was ordered by
  name alone. It is the row every other row is compared against.

And one still open: nothing pings a joined node, so its badge says
"Joined" and never changes. `last_seen_at` is written once, at the join.
The first thing phase 2 makes possible is for that to mean something.

## What phase 7 decided

A name is served by **a set of public nodes**, chosen on the service
page by the node that owns the service — the same hand that decides
where the replicas run. Not one edge: several, because a name that
survives one node going away is the point, and choosing them one at a
time is the same choice made worse.

`service_edge` is the choice (migration `0023`), and it is keyed by
`(hostname, node_id)` rather than by service, because the constraint
that matters is that **one name has one authority**. The receiving node
claims the name before it serves it and refuses a second authority
asking for the same one.

The errand carries **one upstream entry per replica**, not per node.
That is the whole of the load balancing: the edge picks by turn, so a
node running two copies appears twice in the list and receives twice the
requests. Nothing weights anything; the weight *is* the repetition,
which means a replica moving or dying changes the share without anything
recomputing a ratio.

Each entry is the node's **overlay address and a port bound to it** —
`replicas::overlay_port`, from a range disjoint from the published one.
Never the container's own address: that is on a CNI bridge whose subnet
is identical on every node, so `10.88.0.3` names a different container
on each machine reading it. The containers stay where they are, on the
private bridge, reachable only through the port their own node opened on
the overlay.

Three things follow that are easy to get wrong:

- **A node dropped from the list has to be told.** It keeps answering
  for the name, with a certificate, until an errand says otherwise, and
  nothing else in the system would notice. `edges::set` returns exactly
  who was dropped so the caller cannot forget; an empty upstream list is
  the instruction to release the name.
- **The list is re-sent on every placement change**, not only when the
  set of edges changes. A replica that moved is a different upstream
  list, and an edge holding the old one proxies to a container that is
  not there.
- **A replica with no overlay port is left out.** It has reported none,
  so nothing can reach it. An invented address there is a request that
  hangs rather than one that fails over.

## What the nodes said about 4 to 7

**Verified between the two nodes on v0.6.6**, and it took seven fixes to
get there. Every one of them was invisible to a green suite for the same
reason: in a test both ends are one database and the rows are the ones
the test wrote, so nothing ever exercises what one node *knows about
another*.

The measurement that matters, on a service owned by the Ubuntu node with
two copies on the Alpine one and a third at home — packets into each
container's own interface, read from its netns, over 60 requests:

| | replica | packets |
|---|---|---|
| Alpine | slot 1 | +51 |
| Alpine | slot 2 | +51 |
| Ubuntu | slot 3 | +54 |

**1.89 : 1.** The node holding two copies takes twice the traffic, and
nothing carries a weight — it appears twice in the list. Per replica the
split is even, which is what makes the ratio fall out on its own.

The isolation held with it: the local copy is reached at its bridge
address, the remote ones only at `10.42.0.3:30000` and `:30001`, ports
bound to that node's overlay address. No container has anything open to
the world.

The seven, in the order the nodes gave them up:

1. **A joined node is recorded as private** — correctly, by the rule
   enrolment follows — so `may_be_edge` was false and the picker could
   never offer it. Reachability now travels on the report.
2. **A replica that moved kept running where it left.** Reconciliation
   only ever starts things, on purpose, so the stop belongs where the
   decision is made.
3. **The fix for (1) shipped dead**: the report was skipped when it
   carried no replicas, and a node holding none is exactly the one never
   chosen as an edge.
4. **A copy placed elsewhere had no way in.** The console filled the
   errand's port with `None` always, the receiving node made a service
   with no port row, and the overlay port was opened only for a port
   with a hostname *here* — which a derived service never has.
5. **A service running entirely elsewhere is not a failed deployment.**
   The job retried it for ever, once every fifteen seconds.
6. **Routes were not recomputed when the last copy left**, because the
   deployment that recomputes them was correctly skipped.
7. **Nor when a report arrived** with the port a copy answers on
   elsewhere — the deployment that would notice runs on the other
   machine.

Six of the seven are the same shape: **derived state that nothing
recomputes when its input arrives over the network.** On one node the
local reconciliation covered it. With two, the input comes from outside
and nobody was listening.

**Still not run once:** choosing another node as an edge. Everything
under it is verified — the claim, the errand, the route, the release —
and the button itself has never been pressed.

## Phase 8: what is required, and what is offered

Authority is directed, and until now it was also **whole**: joining
granted the enrolling node everything it might ever ask for. That was
never stated as a decision — it fell out of there being only one kind of
errand — and the moment there were two it started lying in both
directions at once.

The Alpine node showed it. Its console offered the Ubuntu node as an
edge for a service of its own, wrote the row, queued the errand — and
the errand will sit there for ever, because Ubuntu takes orders from
nobody. The page says "served by Ubuntu". Nothing is. Nothing ever
will be.

The fix is not to hide nodes that cannot be told. It is to say **what
is being asked for and what is being given**, at the moment the two
nodes agree to know each other:

- The node minting a token declares what it **requires** of the joiner
  and what it **offers** to it.
- The joiner, spending the token, is shown both lists **before** it
  commits, and accepts or refuses. A join is not a click on a token; it
  is a click on a sentence somebody can read.
- Each side then holds only the grants it agreed to, and either can
  revoke one without revoking the rest.

### A capability is a property of the node first

Before it is something one node grants another, it is something the node
**provides at all — to anybody, itself included**. That layer came out
of Jorge's reading and it simplifies rather than adds:

- **Private stops being a category.** A private node is a node that does
  not provide `edge`, which covers the one that *cannot* — no address
  the world can dial — and the one that *will not*, with a perfectly
  good address it has decided not to expose services from. There is no
  "private" switch and there must never be one: naming it that would
  make the other switch look like something else.
- **A node can provide no `host` either.** Small and cheap, owning its
  projects and placing every copy on machines with more room. It is not
  a placement target for its own services, which is exactly what "prefer
  to host elsewhere" means.
- **`Kind` stays derived, never trusted.** Providing `edge` requires the
  endpoint, so a setting can only ever *reduce* what a node claims — no
  switch can make a node look reachable when it is not, which is why
  `Kind` came from the endpoint in the first place.

A capability the node does not provide cannot be granted to anyone. So
the grant negotiation below is bounded by this, and a token cannot offer
what the node has turned off.

### The capabilities

Small on purpose. Each one is a thing one node can ask another to do,
and each is refusable on its own:

| | What holding it lets the other node do |
|---|---|
| `host` | Place replicas of its services here, and pull the images to run them |
| `edge` | Ask this node to answer for one of its hostnames |

`host` carries the registry pull with it rather than being a third
capability: a node that may run your containers must be able to fetch
them, and a grant that cannot be used is not a smaller grant, it is a
broken one.

### What it changes

- **The selectors stop lying.** The placement selector offers nodes
  that granted `host`; the edge picker offers nodes that granted `edge`,
  plus this node itself, which needs no grant to instruct itself.
- **A refusal is visible.** A capability not granted is shown with the
  reason and the way to ask for it, rather than the node quietly not
  appearing — the difference between "there is nobody" and "I did not
  ask them".
- **Revoking is partial.** Today revoking a node's grant is all or
  nothing. A node that will serve your names but no longer run your
  containers is an ordinary thing to want.

Existing joins are backfilled with **both** capabilities. They work
today, and a migration that quietly took something away would break a
running network to make a table tidier.

### The one thing to get right

The joiner has to see the terms **before** the token is spent, which
means the terms travel with the token rather than being asked for
afterwards. A join that showed the terms after committing would be a
consent screen for a decision already made — which is worse than no
screen at all, because it looks like one.

### What revoking turned out to need

Phase 8 built consent and did not build its opposite. Withdrawing
`edge` from a node left behind everything that consent had produced: the
claim on the name, the proxy route, and an ACME order repeating twice a
day for a name the node would never answer for again — against an
authority that locks the account after five failed authorizations. Found
by revoking one on a real pair of nodes, and measured five minutes later
still answering an http-01 challenge for it.

Three causes, each sufficient on its own:

- **The grant was checked before the errand was read.** So revoking it
  blocked the empty-upstream errand that exists to release the name: the
  owner detected the drop, correctly sent the withdrawal, and the node
  refused it — `this node has not agreed to 'edge' for that node`. A
  withdrawal needs no permission. Consent is for taking work on, not for
  putting it down.
- **Nothing convergent released the claim.** The withdrawing errand
  arrives only if the other node is still there, still knows and still
  reaches this one — and a node revoking a grant is quite often doing it
  because one of those stopped being true. `release_ungranted` runs at
  boot beside the other convergent passes and asks only about now: a
  claim whose authority does not grant `edge` today is a claim to
  release, however it was made.
- **Nothing ever deleted an errand-written route.** `retain_proxies`
  skips a row with no `service_id` deliberately — pruning it would take
  somebody else's name off the air every time anything deployed
  locally — and `forget_control_plane` touches only control-plane rows.
  A proxy row with no service was in neither set, so even the
  *successful* withdrawal path had been leaving the route behind since
  phase 7. `routes::forget_for_other` is scoped to exactly that shape.

The lesson generalises past this feature: **every switch that grants
needs the path that ungrants tested with it.** All three of these are
the same omission, and none is visible from the granting side — the node
that revokes sees its own table change and assumes the rest followed.
