# A network of nodes

Public nodes are entry points; private nodes run containers. Any public
node can be an edge for a name whose container lives on a private one,
and a name can eventually be served by a group of them.

This document is the plan and the record of what was decided and why.
Phases 0 and 1 are in the tree; the rest is not.

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

**Kernel when present, userspace when not.** The protocol is the same
and the implementations interoperate, so this is not "proven versus
experimental" — it is a kernel implementation against a userspace one.
Kernel WireGuard is meaningfully faster (no context switch per packet)
and far more rodded. But a one-core node that terminates TLS and proxies
HTTP runs out of CPU somewhere else first, and requiring a kernel module
or `wg-quick` reopens exactly the class of failure Alpine kept
surfacing: no iptables, no cgroups mounted, no overlay module. `boringtun`
is a Rust crate and keeps the one-binary promise.

This is a phase 2 decision and deserves a throwaway spike before
anything is committed to.

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
| 2 | Overlay | Spike, then a real session. `doctor` proves it | next |
| 3 | Errand: host | A queues a deploy on B | |
| 4 | Errand: edge | A tells C to route a name to B; C obtains the certificate | |
| 5 | Groups | Several upstreams per name, health, failover | |

Phase 2 is where the throwaway spike belongs — kernel WireGuard against
`boringtun`, on the Alpine node, before anything is committed to. Every
piece it needs from phase 1 is already in the token: both public keys,
both overlay addresses, and an endpoint.

## Two things not to discover late

(Both still ahead. Phase 1 touched neither.)

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

**Not verified on a node yet.** Everything here passes against the real
migrations and the real router, and none of it has run between the
Ubuntu box and the Alpine one. That is the check phase 1 was chosen for,
and until it happens the certificate requirement in particular is a
claim rather than a fact.

Two things a node will show up:

- The Alpine node answers to a name, so its own row reads *public*. That
  is correct — reachability, not policy — and it means "private node" on
  the enrolling side is about how it was recorded, not about what the
  joined machine thinks it is. Worth watching whether that reads as a
  contradiction on the page.
- Nothing pings a joined node, so its badge says "Joined" and never
  changes. `last_seen_at` is written once, at the join. The first thing
  phase 2 makes possible is for that to mean something.
