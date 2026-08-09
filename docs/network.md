# A network of nodes

Public nodes are entry points; private nodes run containers. Any public
node can be an edge for a name whose container lives on a private one,
and a name can eventually be served by a group of them.

This document is the plan and the record of what was decided and why.
Phases 0 to 3 are in the tree. Phases 0 to 2 are verified between two
real nodes; phase 3 is not yet.

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

**Overlay addresses are `10.42.0.0/16`, lowest free first.** Not
`10.0.0.0/24` or `10.1.0.0/16`, which is what everything else picks by
default and where a collision costs the node its own default route.
Lowest free rather than next, so an overlay whose nodes come and go
stays dense enough to read. A pending token holds its address — it is
already written into something somebody is carrying.

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
| 3 | Errand: host | A queues a deploy on B | **done** |
| 4 | Errand: edge | A tells C to route a name to B; C obtains the certificate | |
| 5 | Groups | Several upstreams per name, health, failover | |

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
