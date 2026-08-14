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

## Two languages

The console reads in English or Spanish, chosen by a toggle beside the
theme and stored on the account. Three rules, and the tests enforce all
three:

**The English text is the key.** `t("Add a port")` returns the Spanish,
or the English back when there is none. Symbolic keys would mean reading
a table to find out what a page says. The cost is that changing a word
orphans its translation — which is the right way round, because an
orphan renders a page somebody can still use, and
`every_string_the_console_asks_for_is_translated` names it by hand.

**The language is scoped around the render, not threaded through it.** A
`Language` parameter on every view, card and row would be hundreds of
signatures carrying it to the leaves. A task-local was the obvious shape
and cannot work: the middleware that knows the account returns *before*
the handler runs, so it has no future to wrap. Each view ends in one
`rsx!{…}.render()` with no `await` inside, so a thread-local set for
exactly that call cannot be seen by another request.

**Commands are not prose.** `docker login`, `wabot-deploy join` and
their arguments render without `t()`: they are pasted into a terminal,
and a terminal does not speak Spanish. The sentence around a command is
translated; the command is not. The same goes for hostnames, ids, image
names, slugs, and the words containerd uses for a container's state.

`doctor` is not translated at all. It runs on a terminal and prints what
somebody pastes into an issue.

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
scripts/cross.sh root@<host> [root@<host>…]   # build here, install there, restart
ssh root@<host> 'journalctl -u wabot-deploy -f'
ssh root@<host> wabot-deploy doctor
```

**One static musl binary, built here, for every node.** `cross.sh` uses
`zig cc` through `cargo-zigbuild`; the prerequisites are
`brew install zig protobuf` and `cargo install cargo-zigbuild`.

This replaced building **on** the node, which was the rule here for a
long time and for a good reason — it removed the whole class of "the
binary does not run there". Three things ended it, and the third is the
one that cannot be worked around:

- **It stopped fitting.** `rustc` on the final crate was OOM-killed on
  the one-core box (2026-08-12), and had taken `clippy-driver` three
  days earlier. It builds again with `CARGO_PROFILE_NODE_LTO=false` and
  sixteen codegen units, which is the profile degraded to fit.
- **It takes the machine away.** sshd cannot fork while the build holds
  the memory, so for three or four minutes the node answers HTTPS and
  refuses logins — and once, for two and a half hours. Use the console
  over HTTPS to tell "busy" from "gone"; retrying SSH only adds load.
- **One binary cannot serve both nodes.** Ubuntu is glibc, Alpine is
  musl. Building on each is two artifacts, and the Alpine box could not
  build this one at all — 972 MB of RAM, and it filled all 512 MB of its
  swap on the final crate.

Cross-building is ~2 minutes on eight cores with the profile *intact*,
and both test nodes now run the identical artifact — verified by sha256
on each. `deploy.sh` is still there for building on a node when that is
what you want; it now knows `apk` as well as `apt` and `dnf`.

Two things it does not do: it does not touch the systemd unit, and it
keeps the binary it replaced at `/usr/local/bin/wabot-deploy.previous`,
which is the way back if the new one does not start.

**And check `/tmp` before believing a node is out of memory.** It is
tmpfs. A forgotten source tree with its `target/` sat there for three
days holding 702 MB — 37 % of the machine — and that, not the size of
the binary, is what had made building impossible.

Never fabricate a session or a token in the node's database to test a
page. Ask for the click. (`wabot-deploy passwd <username>` exists for the
operator who is locked out of their own node — that is recovery, not a way
to conjure a session for a test.) That is also why **anything visual has to be
looked at by somebody**: without a session the console answers 302, so
four UI faults in one afternoon — a repeated word, a hard-to-hit
control, a misaligned icon, a password manager hijacking a field — were
all found by Jorge and none by the tests.

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
- `scripts/deploy.sh` knew `apt-get` and `dnf` and not `apk`, so a
  build on the Alpine node failed at the preparation step asking for
  `build-essential`. Fixed; `build-base protobuf-dev` is what that
  machine calls them. Building there is no longer the way in — see
  "Working on the node" — but the script is still the one that does it.
- `scripts/deploy.sh` prints `line 76: release: command not found` on
  every run. Harmless, and it is exactly where somebody looks when
  something is wrong.
- **An errand a node queues for itself is never collected.** The
  collector asks its *authorities*, and a node that takes instructions
  from nobody has none — so a row addressed to this node sits pending for
  ever. One is on the Ubuntu test node from 2026-08-13. Harmless and
  confusing, which is the combination that costs somebody an hour.
- `doctor` prints the overlay port from the config under a comment
  promising what the kernel says. The peers below it are read from the
  kernel; the port is not.
- **Two design-system fixes are held in `layout.rs` rather than in the
  vendored stylesheet**, and both belong upstream: the muted and faint
  ramps, which failed contrast for body text, and a checked checkbox
  whose mark was a fixed white on a box coloured `--c-fg` — cream on
  cream in dark mode, a checked box indistinguishable from an empty one.
- A value the stream assigns has to be the shape the CSSOM takes, and
  the CSSOM refuses silently: `style.width = "width:12%"` does nothing
  at all. The meter on the memory page froze that way for as long as it
  has existed, and the numbers beside it kept moving, so it read as
  working. There is a test on the payload's shape now.

**Nothing asks a certificate authority for a name that does not point
here.** The check existed — `dns::resolves_here`, with four answers and a
sentence for each — and it was asked in one place: the form where a
hostname is typed. A hostname is typed once; the renewal loop runs for as
long as the node does. So a domain that expired, or a record somebody
repointed, was two failed validations a day against an authority that
locks the account at five an hour — and the lock is per account, so one
moved name takes every other name on the node with it. `acme::ensure`
asks before it spends anything, after the freshness check so a certificate
that needs nothing costs no lookup.

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

**A join states its terms, and the joiner reads them first.** The token
carries what the minting node *requires* and what it *offers*, per
capability; the joining console shows both lists and lets each required
one be ticked or refused before the token is spent. That order is the
whole point — terms shown after committing are a consent screen for a
decision already made, which is worse than none because it looks like
one. `wabot-deploy join` passes `None`, meaning "whatever it asked":
there is nobody at a terminal to show a screen to, and typing the
command is the consent.

Two capabilities, `host` and `edge`, each granted and revoked on its
own, and **two tables, because the answer has to travel**:

- `node_grant` is what **this** node agreed to do, for whom. It lives on
  the machine the decision binds, like the authority row and for the
  same reason, and it is what `carry_out` checks before obeying an
  errand — a grant nothing enforces is decoration.
- `node.allows` is what **another** node lets this one ask of it. A
  learned fact on that node's row, beside `endpoint`, arriving on the
  join callback and refreshed by every report. It has to be learned:
  the decision is a row on the other machine and this one has no access
  to it, which is the trap the first version of this walked into — the
  selector read the local grant table, which answers the opposite
  question.

A grant is read through what the node provides *now*, so turning a
switch off withdraws what was granted of it — the switch is the more
recent decision.

Both selectors read the grants. A node that never agreed to run your
containers is not somewhere you can place a replica, and offering it
produced an errand nobody would ever collect — which is exactly what the
Alpine node did, silently, while its page said the name was served.

**Revoking has to work, and it took three fixes to.** Jorge withdrew
`edge` from one node, and everything that consent had produced stayed
exactly where it was: the claim, the proxy route, and a Let's Encrypt
order repeating twice a day for a name that node would never answer for
again. Three separate causes, and each one alone was enough:

- **A withdrawal needs no permission.** The grant was checked before the
  errand was read, so revoking it blocked the empty-upstream errand that
  exists to clean up — the owner did the right thing and was refused.
  Consent is for taking work on, not for putting it down.
- **Nothing convergent released the claim.** The withdrawing errand
  arrives only if the other node is still there, still knows and still
  reaches this one — and a node revoking a grant is often doing it
  because one of those stopped being true. `network::release_ungranted`
  runs at boot and asks only about now.
- **Nothing ever deleted an errand-written route.** `retain_proxies`
  skips a row with no `service_id` on purpose and
  `forget_control_plane` touches only control-plane rows, so the proxy
  row an errand writes was in neither set. Even the *successful*
  withdrawal path had been leaving it behind. That is
  `routes::forget_for_other` now.

Verified on the node: the claim, the route and the order all gone at the
next boot, `no problems found`.

**A stop has to travel, and stopping needs the same permission as
placing.** `stop` took down the copies here and said nothing to the
machines running the others, so a service the console called stopped went
on serving traffic elsewhere. The intent rides beside the placement now —
`Host.running` and `Standby.running`, defaulted true — because `slots:
[]` already means something else: "take this off that machine", which
deletes the rows there. And the phase 8 rule does **not** carry over:
withdrawing an *edge* needs no permission because it only asks a node to
stop answering for a name, where this reaches in and stops processes. So
the grant is checked, the dispatch skips a node that no longer allows it,
and the hole that opens — a node that revoked `host` cannot be *told* to
stop — is closed by that node itself: `evict_ungranted` runs at boot
beside `release_ungranted` and throws off what it no longer consents to
run. Evicted rather than stopped, because somebody did throw it out, and
the report is how the owner learns to stop asking.

**"Not deployed" was the page answering about the wrong machine.**
`observe_service` asks this node's runtime, so a service placed entirely
on other nodes read as absent. It has two answers now — `Running
elsewhere` once a copy has reported an address, `Waiting for that node`
until one has — and its action is `stop`, which is what the owner of a
service on somebody else's machine can do about it. `deploy` no longer
refuses a service with no copy here; it tells the holders.

**A report that says the same thing is not a change.** `api::record`
answered `true` whenever it wrote, and `true` means "something moved":
the authority rebuilt its routes, rewrote every container's `/etc/hosts`
and woke the certificate loop every fifteen seconds for as long as both
nodes were up — 41 rebuilds in ten minutes, and an ACME loop that never
reached its twelve-hour wait. The comment above the call site said
exactly what should happen.

**A restart must not touch a healthy overlay.** Nothing takes `wabot0`
down when this process stops — the interface, its peers and the kernel's
sessions outlive the binary, and so does the port mapping, which is
iptables — so packets keep crossing while a node is being replaced. The
only thing that can break that is `tunnel::apply`, and it did:
`configure_interface` sets `ReplacePeers`, which drops every session key
and every *learned* endpoint, costing 45–55 seconds against a
`wal_receiver_timeout` of 60. Every deployment cost the remote standby
its replication stream. It compares first now — and the comparison itself
took two node runs: **a private key read back from the kernel is not the
one in the database** (WireGuard clamps a secret when it takes it, so
public halves are what to compare), and **a keepalive of nought is no
keepalive**. What made both findable was making the decision speak: the
rebuild logs its reason, and the quiet case logs `the overlay interface
already matches`.

Next: phase 9, groups — health and failover across the upstreams of one
name. Today a dead replica keeps its share of the traffic until the
owner notices.

## Databases

[`docs/databases.md`](docs/databases.md) is the plan and the record.
Postgres first, on top of `service` and `replica` rather than beside
them: slot 1 accepts writes, every other slot follows it.

Three things it needed that the platform had never had, and all three
are generic:

- **A volume.** `containers::run` removes the container and its
  snapshot before it starts, so everything a container wrote is gone at
  the next deployment — right for a stateless service and total loss
  for anything else. `platform::volumes`: the row is the service's, the
  directory is the *replica's*, and the directory is derived from the
  container id rather than stored.
- **A memory ceiling.** Nothing ever wrote `linux.resources`, so a
  container could take the machine. `service.memory_limit` reaches the
  spec as `memory.max` with swap off, and `/dev/shm` — which was a
  hard-coded 64 MB — follows it.
- **`ContainerRequest.args`.** Appended to the image's own command,
  where `command` replaces it. That is how `postgres -c
  shared_buffers=32MB` keeps the entrypoint that runs `initdb`.

**A preset is not only a cgroup limit.** It also sets the engine's
arithmetic, because 64 MB of ceiling with the stock `shared_buffers` of
128 MB is a container killed before it starts. `platform::postgres` is
pure and holds the table.

**Two traps the errand path had, both found writing this:**

- A `host` errand minted a token for *this* node's registry and sent it
  to whatever host the image named — so placing
  `docker.io/library/postgres` elsewhere would have handed a wabot push
  token to Docker Hub. The credential is optional now and is minted
  only when the registry is this node's own.
- **The overlay is a star, not a mesh.** A join writes one row on each
  side, so two nodes enrolled by the same authority have never heard of
  each other and have no WireGuard session. Phase 7 did not notice
  because the verified topology always had the authority at one end. A
  standby dialling a primary on a third node is the case with no path,
  and the fix — the peer travelling in the errand — is phase 4 there.

**Phases 0 to 2 are verified on the Ubuntu node** (2026-08-12, a 256 MB
PostgreSQL 17 created from the console): `memory.max` and
`memory.swap.max` read back out of the cgroup, `shared_buffers` and
`hba_file` read back out of the running server, `initdb` into the bind
mount, the reserved address held across a redeployment, and a row
written before a `SIGKILL` still there afterwards with `PG_VERSION`
untouched.

**Phases 3 and 4 are verified across both nodes** (2026-08-13): a
standby beside the primary and a second on the other machine, both
`streaming` in `pg_stat_replication`, the remote one read-only, and
`sslmode=verify-full` against the database's own qualified name working
on either. Phase 5 — noticing a standby that stopped following — is not
written.

**A database's name is the operator's, and what the page offers is what
verifies.** The name was derived from the node's domain, which is the
operator's choice only while every name is a subdomain of the node — it is
a field now, on `port.hostname` where a service's lives, and the read
pool's is the primary's with `-ro` in the first label. `docs/naming.md`
carries the rest, and two things there are worth knowing before touching
this:

- **A certificate can be current, from the right authority, and wrong.** An
  ACME order was one identifier, so choosing Let's Encrypt for a database
  named the primary and not the pool, and every read would have failed
  `verify-full` against a certificate with three months left. Freshness
  cannot see a missing name; `ensure` compares the name set now, which
  `ensure_self_signed` always had.
- **A public authority cannot sign a short name**, because nothing outside
  the node resolves it — and a node with no domain has no long name at all.
  So the console offers both spellings only where both are on the
  certificate, and never a string `verify-full` would reject.

**A name belongs to the database, not to the machine holding a copy.**
The long name used to be built from the local node's domain for
everything on the node, so the same database answered to a different
qualified name on each machine and its certificate matched neither. The
owner's domain travels on the errand and lives on the row
(`database.owner_domain`); `hosts::entries_for` takes the suffix per
service, and `certificate_names` prefers the row's. `docs/naming.md`
has the measurement.

That fix then reached nobody, twice, and both are worth carrying:
`adopt` bound the value and left the column out of the `INSERT` — **a
parameter nobody uses is not a warning** — and the errand was recomputed
only by a deployment, so a payload that gained a field stayed the
payload from before the upgrade. Reconciliation dispatches the standby
errands at boot now, which is phase 7's lesson again.

**Two bugs came out of the node, and both were invisible locally:**

- **`CNI_ARGS=IP=` does not reach `host-local`.** The `bridge` plugin
  parses `CNI_ARGS` into a struct that knows `MAC=` and nothing else,
  and a key it does not recognise is a hard error — `ARGS: unknown
  args`. A wanted address travels in `args.cni.ips` inside the config.
  Every database's first deployment would have failed.
- **A requested address must be inside a configured `host-local`
  range**, not merely inside the subnet. Bounding the allocator to the
  low band and reserving the high one out of its sight — the two-port-
  range construction — is refused with `requested IP … not in range
  set`. Both bands are declared, in one range set, low first.
