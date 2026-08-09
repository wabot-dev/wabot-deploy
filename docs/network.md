# A network of nodes

Public nodes are entry points; private nodes run containers. Any public
node can be an edge for a name whose container lives on a private one,
and a name can eventually be served by a group of them.

This document is the plan and the record of what was decided and why.
Phase 0 is in the tree; the rest is not.

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

From a public node: "add private node" mints a token and an install
command. The token carries the public node's endpoint, its WireGuard
public key, the overlay address it allocated, and a bearer secret.
`wabot-deploy join <token>` is the sibling of `install`.

The pattern already exists twice — `setup-token` and people invitations
are both "a single-use secret, shown once" — and the push-token fix from
the same work is the template for showing it without putting it in a URL.

Two things to decide before writing it:

- **That token is a network credential.** Single use, short lived, shown
  once, hashed at rest.
- **`curl | sh` is outward-facing.** It is the convention and it is
  convenient; it also defines how this product is distributed.

## Phases

| | Phase | Delivers | State |
|---|---|---|---|
| 0 | Model | `node`, `authority`, `claim`; the claim rule | **done** |
| 1 | Enrolment | Token, `join`, keys, B recording A as authority. No new networking | next |
| 2 | Overlay | Spike, then a real session. `doctor` proves it | |
| 3 | Errand: host | A queues a deploy on B | |
| 4 | Errand: edge | A tells C to route a name to B; C obtains the certificate | |
| 5 | Groups | Several upstreams per name, health, failover | |

Phase 1 has the best return for its size: end-to-end verifiable with the
Ubuntu node as public and the Alpine one as private, and it needs no new
networking at all.

## Two things not to discover late

**Images live where they were pushed.** The registry is per node, and a
private node needs the image to start the container. Either you push to
each, or the registry replicates, or private nodes pull from a public
one's. That is design, not detail.

**Deploying is local.** `deploy` talks to *this* node's containerd. An
errand to host something has to run its job on the target node, so the
queue stops being purely local and job routing appears.

## Where phase 0 stopped

`migrations/0015_network.sql` and `src/network/mod.rs`, with tests.
Nothing consumes it yet, so the module carries an `allow(dead_code)`
naming what removes it:

1. `install` seeding the self row.
2. The console reading this table instead of `node::all()`'s synthetic
   list of one.

That wiring is the first half-hour of the next session, and it is what
makes the model real rather than described.
