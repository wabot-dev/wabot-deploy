# wabot-deploy

A single-node container platform: one binary that installs itself on a
Linux box, terminates TLS, receives images, runs containers and updates
itself. Read [`README.md`](README.md) for what it does and
[`docs/architecture.md`](docs/architecture.md) for why it is built this
way — §10 there is the record of what went wrong and what it taught.

## The gate

```sh
cargo fmt --all --check
cargo clippy --all-targets -- -D warnings
cargo test
```

All three, before saying anything is done. CI runs the same three, and
so does the release workflow — a tag can point at a commit CI never saw.

They are only the same three if the compiler is the same one.
`rust-toolchain.toml` names a **version**, not `stable`: it used to say
`stable` and claim to pin, and every machine resolved that to whatever
it had while CI installed the newest on every run. A lint added between
two releases then fails the gate on CI having passed it locally. Raising
it is a deliberate edit — change the number, run the three, fix what the
newer compiler found.

`protoc` must be installed: `containerd-client` generates its gRPC
bindings in a build script.

## The framework is ours

This is built on [wabot-rust](https://crates.io/crates/wabot), which we
own. When something is missing, the question is not "how do I work
around this" but **"is this a generic capability the framework lacks, or
is it wabot-deploy's own?"** If the first, it goes upstream and every
wabot app inherits it — that is how `#[raw]`, TLS, SQLite, cooperative
cancellation and `#[patch]`/`#[head]` got there.

The checkout lives at `../../framework/wabot-rust`. Changes there need
their own tests, a version bump and a publish before this repo can use
them; `Cargo.toml` depends on the published crate, not a path.

## Conventions the code already follows

**Comments say why, not what.** The code says what it does. A comment
earns its place by recording the alternative that was rejected, the bug
that made this necessary, or the thing that will look wrong to the next
reader. Match the density of the file you are editing.

**Tests state a claim, not the implementation.** Name them as sentences
(`a_stale_run_does_not_block_the_next_one`). A test that restates the
body of the function it tests passes forever and catches nothing — that
happened here once, and CI caught the drift. When a test guards against
a bug that shipped, say so in a doc comment on the test.

**Verify against the node, not against the model in your head.** Almost
every hard bug in this project was invisible locally: the mount
namespace, the CNI teardown, the HTTP/2 downgrade, the staging
certificate that looked fine. If a change touches containerd, systemd,
CNI or ACME, it is not verified until it ran on a real node.

**The ledger records; it does not decide.** Install steps are
convergent — each one asks about the thing, not about the history. This
rule was learned three times.

**A boolean attribute is written by branching, not by a value.** `rsx!`
renders `hidden=(false)` as `hidden="false"`, and HTML reads `hidden`,
`disabled`, `checked` and `selected` by *presence* — so that hides,
disables, checks and selects. Write `@if cond { <el attr> } @else { <el> }`.
This was wrong in four places at once, and each was invisible: a button
that appeared a second late, a checkbox that always read on, a selector
naming the wrong project.

**Errors are values somebody can act on.** A failure that only reaches
the journal is a failure nobody sees: put the reason where the person
looking for it will be (the run row, the service row, `doctor`, the
page).

**The console works without JavaScript.** That is the rule; "no
JavaScript" was the proxy for it, and the proxy started costing more
than it bought. Somebody opens this console when the node is unhealthy,
so every page must render complete and every form must submit with
scripting off.

**The framework's client runtime is already there.** `/_wabot/client.js`
comes with the `ui-hypertext` feature, every page loads it, and
**boosted navigation is on**: an in-console link swaps the view with
`innerHTML` rather than loading a page. Two consequences, both of which
shipped as bugs before anyone noticed:

- A `<script>` inside swapped HTML **never runs**. Inline scripts in a
  view are dead on arrival for anyone who clicked a link.
- A listener attached once at load belongs to a form the next swap
  discards.

So client behaviour goes through `wabot.island(id, mount)` and a
`<wabot-island>` host, which the runtime re-hydrates after every swap
and tears down when the host leaves. `assets/console.js` registers the
two this console has: `fields` (hides what does not apply, adds
`required` where the server already refuses) and `node-live` (the
stream). Both declarative — the markup carries `data-when` /
`data-required-when`, so a new dependent field is an attribute rather
than another script.

Preact is available through `wabot-ui-bundler`, and deliberately unused:
it is a build dependency wanting Node and esbuild, and `deploy.sh`
builds *on* a one-core node. Reach for it when something needs real
component state — nothing here does yet.

An island may **hide what is irrelevant** and **add a constraint the
server already imposes**. It may not be needed to submit a form, fetch
data the page did not have, or build a field. A rule the browser
enforces is a courtesy; the check that counts is on the POST.

## Design

The console follows the Wabot design system: no borders, no shadows, no
hover state changes, primary actions are black, brand orange is for
highlights only, sentence case, no emoji as iconography. Status is a
coloured dot plus a word. `src/console/layout.rs` holds the page-level
CSS; the tokens come from `assets/`, vendored so a node never needs a
CDN.

## Working locally

```sh
scripts/dev.sh                           # https://localhost:8443, installs on the first run
scripts/dev.sh --reset                   # throw the local node away
```

For console work. Pages, sessions, the registry and the database are
the real ones; containerd is not there, so every service reads
`unknown` and nothing deploys. Everything above the runtime is what a
node runs, and it comes up in seconds.

State lives in `.dev/`, which is gitignored — deleting it is the reset.
The certificate is self-signed, so the browser warns once; trust
`.dev/data/certs/local-ca.crt` to stop it. ACME is off by default here
(`WABOT_DEPLOY_ACME_DISABLED`), because a laptop that sets a domain
would otherwise place real orders and production locks the account out
after five failures.

This is not the verification step. Containerd, systemd, CNI and ACME
still have to run on a node — see the rule below.

## Working on the node

```sh
scripts/deploy.sh root@<host>            # builds on the node over SSH, installs the binary
ssh root@<host> systemctl restart wabot-deploy
ssh root@<host> 'journalctl -u wabot-deploy -f'
ssh root@<host> wabot-deploy doctor
```

It builds **on** the node because that removes the whole class of "the
binary does not run there". Two things to respect: the test node has one
core and no swap, so a `--release` build there takes the machine down —
`deploy.sh` uses the `node` profile for that reason — and `deploy.sh`
only installs the binary, it does not restart the service.

Never fabricate a session or a token in the node's database to test a
page. Ask for the click.

## Releasing

```sh
# bump version in Cargo.toml, commit, then:
git tag -a vX.Y.Z -m "…" && git push origin main vX.Y.Z
```

The tag is the trigger and the tag is the version; the workflow refuses
to build if `Cargo.toml` disagrees. It produces one static musl binary
plus its checksum, which is what the console's updater downloads.

Publishing a release is outward-facing. Ask first.

## Things that are still open

- Reconcile checks whether a container runs, not whether its port
  mappings match the rows.
- No image garbage collection.
- The updater does not rewrite the systemd unit; a release that changes
  it has to say so in its notes.
- No rollback button for an update — rolling back a migration is not a
  file operation, which is what the database copy is for.
- Wildcard certificates are refused by name: the resolver looks names up
  in a map, so accepting one would store a certificate never served.
- Building happens on the node, ~25 minutes on the one-core box.
  Compiling locally against a static musl target is the clean way out,
  and it contradicts the "Working on the node" rationale above — so it
  needs that section rewritten, not quietly ignored.
- `scripts/deploy.sh` prints `line 76: release: command not found` on
  every run. Harmless, and it is exactly where somebody looks when
  something is wrong.

## The network work

[`docs/network.md`](docs/network.md) is the plan and the record of what
was decided. Phases 0 and 1 — the model, and enrolment on top of it —
are in the tree: a public node mints a join token, the joining node
records it as an authority and calls back, and the nodes page lists the
`node` table. A token can be spent from the joining node's console or
from its terminal — `network::join` is the one implementation, and both
doors are thin over it.

Two things about it that are easy to get wrong from the outside:

- **A node's id is its own.** Minted at `install`, kept for ever, and
  what every other node calls it. The enrolling node allocates the
  overlay address and nothing else.
- **Nothing travels between nodes yet.** The one call that exists goes
  *from* the joining node *to* the authority, once, at join time.
  Errands need the overlay, which is phase 2.

Phase 1 is verified between both test nodes on v0.3.0. Three things only
that showed — see the end of `docs/network.md` — and the one worth
carrying forward: **a row about another node describes the relationship,
not the machine.** A joined node may well be public and say so on its own
page; what the enrolling node knows is that it has nothing but an overlay
address for it.

Phase 2 — the overlay — is written, and its spike **reversed** the
decision the plan had recorded: the `wireguard` module is in both nodes'
kernels, and on Alpine `/dev/net/tun` is not there at all, so userspace
WireGuard would swap one module for another and add the data path. It is
kernel WireGuard, configured over netlink from inside the binary, with
no `wireguard-tools` on the node.

Verified between the two nodes on v0.3.1: the module autoloads from the
netlink interface creation, the handshake crosses, and ICMP and TCP both
run over it. `doctor` reads the peers back from the kernel, so one that
has never shaken hands says so — the failure that has to be
distinguishable from working.

Phase 3 — errands — is written. Two shapes worth knowing before reading
it: the node **collects** its errands rather than the authority
delivering them (an authority cannot reach a private node over TLS), and
an errand is an *instruction*, not a job — the node that collects one
writes its own service row and its own local deploy from it.

A node that joined before v0.4.0 has no credential to ask with and must
**re-join**: the direction inverted, and a hash cannot be presented.

**Read `docs/network.md`'s "What it is for" before the phases.** A
service is administered from the node where it was created, and what
lands on a receiving node is derived: not editable there, evictable
there. That reshaped the plan — phase 3 works and its form is on the
wrong page — and it makes a **replica** the unit rather than a service.
A container id therefore has to carry a replica index, which reaches the
runtime, the boot reconciliation and routing.

Phases 4 to 6 are written: a **replica** is the unit (a service is *n*
placements, and a slot number belongs to the service rather than the
node it lands on), what arrives on an errand is not editable where it
lands, the placement page lives on the service, and the node running a
replica reports its state back to the node that placed it — and can
throw it off.

Phase 7 — the edge — is written too. The node that owns a service picks
which public nodes answer for each of its names, and they are told where
to proxy. Three things about it:

- **The errand carries one upstream per replica**, not per node. That is
  the whole of the load balancing — the edge picks by turn, so a node
  running two copies appears twice and gets twice the requests. The
  weight *is* the repetition; nothing computes a ratio.
- **An upstream is the node's overlay address and a port bound to it**,
  never the container's own. A CNI bridge subnet is identical on every
  node, so a container address names a different container on each
  machine that reads it. The containers stay on the private bridge.
- **A node dropped from the list is sent an errand too**, with an empty
  upstream list. It keeps answering for the name, certificate and all,
  until something says otherwise — `edges::set` returns who was dropped
  so the caller cannot forget.

**Phases 4 to 7 are verified between the two nodes on v0.6.6.** A
service owned by one node, two copies on the other and a third at home:
the node with two took 1.89× the traffic of the node with one, measured
as packets into each container's own interface. Nothing carries a
weight — a node appears once per replica, and the ratio falls out.

**Getting there took seven fixes, and no test could have found any of
them**: in a test both ends are one database and the rows are the ones I
wrote, so nothing exercises what one node *knows about another*.
`docs/network.md` lists all seven. Six share a shape worth carrying into
phase 8:

> **Derived state that nothing recomputes when its input arrives over
> the network.** On one node the local reconciliation covered it. With
> two, the input comes from outside and nobody was listening.

The two that will catch the next person: a joined node is recorded as
*private* and stays that way until it reports otherwise, and a service
that arrived on an errand has **no hostname here** — the name belongs to
the node that placed it, so anything keyed on "has a hostname" silently
skips every derived service.

Phase 3 taught what a node run is worth. This was the second and third
time.

**Being an edge is a row, including for the node that owns the
service.** It used to be automatic — this node routed every hostname on
its own ports — and that read the model backwards. The only thing
separating a private node from a public one is **whether it exposes its
own address**, so a private node can own projects and services perfectly
well and have them served from somewhere else entirely. Three things
follow, and all three were wrong before migration `0024`:

- `routing::sync` builds a route for one of this node's own names only
  if `service_edge` says this node answers for it.
- `acme::wanted_names` asks for a certificate on the same condition. A
  private owner ordering one for a name pointing at another machine is a
  challenge that cannot pass, twice a day, against an authority that
  locks the account after five failures.
- `ports::create` writes the default row itself rather than the console
  doing it. A caller that made a port and did not know to write it got a
  hostname stored, shown, and served by nobody — which is what the first
  version did, caught only because a routing test asserted the route.

Next: phase 8, groups — health and failover across the upstreams of one
name. Today a dead replica keeps its share of the traffic until the
owner notices.
