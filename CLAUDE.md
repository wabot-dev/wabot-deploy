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
rule was learned three times, and a fourth outside `install`:
`restore-node` refused to run while the node was up by asking
`is_active`, and `rc-service stop` returns `[ ok ]` with `status:
stopped` while the daemon drains for another seven seconds. It asks the
process table now. A service manager's answer is a ledger entry like any
other.

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
coloured dot plus a word.

**`assets/wabot.css` is ours.** It came down here so a node never needs
a CDN, and *that is also what makes it the place to correct*. Nothing
syncs it back — no script, no build step, no CI job touches it — so the
comment that used to say a sync would silently take a fix away was
describing a hazard this repository does not have. Six corrections
lived in `layout.rs` because of it, restated at a specificity that
could win, and one of them was a *patch for a cause* sitting two
hundred lines from the cause.

So: what a token **is** goes in `assets/wabot.css`, beside the value it
replaces, under a `CORRECTED` comment saying what was wrong and what it
measured. Page CSS — the shell, the meter, the cards, anything this
console invented — stays in `src/console/layout.rs`. The split is the
thing, not the file's provenance.

Two tests read the stylesheet directly rather than a copy of it:
`the_two_dark_palettes_agree`, because dark is written twice and CSS
cannot make a media query set an attribute, and
`a_classed_submit_is_not_the_primary_button`, because
`button[type='submit']` outranks every `.btn-*` variant and takes them
all with it.

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
- ~~Container logs are kept across restarts and bounded three ways, and
  nothing **searches** them.~~ Done: a GET form over the whole kept
  history, matches paged, each opening in context. Substring and not a
  pattern — a regular expression is a way to hang a one-core node from a
  form. A line number counts over the concatenation, so a sweep that
  drops the oldest generation shifts them: it is a place, not an address,
  and the page says the number it counted at.
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
- `doctor` prints the overlay port from the config under a comment
  promising what the kernel says. The peers below it are read from the
  kernel; the port is not.
- A value the stream assigns has to be the shape the CSSOM takes, and
  the CSSOM refuses silently: `style.width = "width:12%"` does nothing
  at all. The meter on the memory page froze that way for as long as it
  has existed, and the numbers beside it kept moving, so it read as
  working. There is a test on the payload's shape now.

**Timestamped logs are a switch, and the switch shows its price.**
`file://` is the default: containerd appends the container's bytes and no
process sits between them, which is why it cannot stamp a line or say
which stream it came from. `binary:///path` hands the streams to a
program instead — `wabot-deploy log-writer`, one process per copy. The
protocol is measured and written down in `commands::log_writer`, including
the part that is a trap: fd 3 is stdout, fd 4 is stderr, and **fd 5 is a
readiness pipe containerd blocks on until it closes** — a logger that
reads only 3 and 4 hangs container creation for ever, with no error
anywhere.

Off by default per service, because it reintroduces the hazard that chose
`file://` in the first place: a pipe nobody drains fills up and blocks
the container's next write. So the writer has **no `?` in its loop** — a
failure drains and discards, because losing lines is bad and hanging a
service is worse.

**A cost stated on a form has to be one the machine agrees with.** The
first version of this said "under 1 MB per copy", which is true of
private memory (0.32 MB measured) and would have been called a liar by
`top`, where each logger reads 8.1 MB — almost all of it this binary's
own text, shared with every other `wabot-deploy` process including the
node. Both numbers are on the form now. So is the disk one: the prefix
is 28 bytes against a mean line of 148 measured here, so the same
retention budget holds about a sixth fewer lines. **Timestamps are paid
for in history**, and that is the sentence somebody needs before they
turn it on, not after they go looking for a log that is no longer there.

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

**"Delivered" is not "done", and consent is checkable at the door.** Two
rules that came out of one chain of five faults — a service on a private
node served by a public one, which did nothing at all. `hand_over` settles
an errand the moment the call returns 200, which only means it arrived:
what the far side did with it never travels back, so `queue_if_changed`
must not read a settled hand-over as carried out, or fixing the cause of a
refusal changes nothing and the identical instruction is never sent again.
And a refusal *for lack of permission* needs one local read, so it belongs
in the answer rather than in a row nobody on the other side will ever see —
`errand::consent_for` is that map, in one place because the door and the
obeying must not disagree. The other three: an edge instruction nothing
recomputed (it hangs off `tell_holders` and off boot reconciliation now,
like a placement), a grant a token *offered* and nothing ever wrote
(`honour_offers`), and a domain that was a private address, which had the
renewal loop asking a public authority for an IP identifier every minute.
`docs/network.md` has all five.

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

**Phase 10 closed the direction that had no path.** Collection asks a
node's *authorities*, so an errand addressed to one was asked for by
nobody — it sat pending for ever and what it asked for never happened.
Two shapes, both found on the nodes: a node's own (being an edge is a
row like any other, including for the owner) and a node's authority (a
service owned by Alpine, served by the Ubuntu node that enrolled it —
five days pending, shown as served, answered by nobody). Its own runs in
the same pass; its authority is handed over, on the road that always
worked. **Queued on arrival, never obeyed in the handler**: a request
can time out and be retried, and obeying writes rows and starts
deployments. And `errand.from_node_id`, because the asker decides two
things obeying cannot guess — the capability check is about *them*, and
the rows belong to them. How it went does not travel back: the asker is
the one that dials.

**Phase 11 is what makes a node manageable from another one, and almost
all of it fits roads that already worked.** A node's own version did not
travel at all, so a console could not say which of its machines was
behind. Now it does, and `manage` — one capability, not backfilled — lets
another node install a release here, restart the service and take a
backup. Five things about it:

- **"Be on this release" is desired state**, so it is an ordinary errand
  on the pull path, in both directions, with no credential and no channel
  behind it. A node already there settles it as satisfied; one that was
  off converges when it comes back. The install ends by replacing the
  process that would have settled the errand, so the settlement can always
  be lost — **the guard is the version, not the order**.
- **How the install went cannot travel that way**, because the errand is
  settled before the binary is touched. `node.update_note` carries one
  line from the run row on the machine that tried it; without it a failed
  download reads as "asked for v0.7.1, still on v0.6.6".
- **Restart and back-up are events, and there the ledger decides.** Asked
  twice they happen twice, and a restart kills the process that would have
  reported it done — so the errand's id is written to `setting` *before*
  the thing is done. It is the one exception in this codebase and it says
  so where it is.
- **Reading a copy's log on another machine** needed the credential phase
  9 left open. It is **derived, not stored** — `sha256(key :
  authority_id)` — so the verifier keeps nothing to be read, can hand it
  out again on any report, and withdraws every one at once by rotating the
  key. It travels on the report; `api::caller` takes either credential,
  because either node may be the one holding the copy. The guard on the
  log is the service's `origin_node_id` and nothing else: reading what
  your own container wrote comes with `host`, like the registry pull.
- **Owning things is a switch and not a capability.** `capability::owns`
  off makes a node purely somewhere resources live. It is outside the
  `Capability` enum on purpose — every variant of that is something
  *another* node asks for, and this is a decision nobody asks for, so it
  has no grant and belongs on no terms screen. Refused while the node
  still owns a project, because those would be neither editable nor
  evictable. Accounts stay: a node nobody can sign in to cannot have the
  switch turned back on.

**And the doorbell is authenticated**, which phase 9 wrote down as owed:
it asks the caller who it is rather than believing the id in the path. A
node too old to present a credential is refused and its rings stop
shortening the wait, which is the whole cost. It also fixes a check that
asked "is the caller one of my authorities" — a *member* is not, so phase
10's handed-up errands rang, were declined, and waited out the loop.

Two things it fixed on the way past. **The capabilities form drew two
checkboxes while the handler wrote a setting for all three**, so saving it
turned `store` off in silence — the node stopped being a standby target
and refused the errands for the copies it held. It is generated from
`Capability::ALL` now, so the next one is a compile error. And migration
`0050` deletes the value it wrote, because **`store` never had a control**:
every stored `off` for it is the bug's handwriting rather than a decision,
which is what makes undoing it a repair and not an overreach. And **grants
could only be written by a join**: there is a form per node, and the
withdrawal it writes is a row of its own, because `honour_offers` re-adds
what the enrolment offered at every boot.

A node older than this refuses the instruction with a sentence about
versions — which is exactly the node somebody wants to update. **The first
hop is manual**, with `cross.sh`.

**Verified on both nodes** (2026-08-23, identical artifact by sha256, the
private one installed through the public one as a jump host): the version
travelling, the inbound credential answering a reasoned 404 where nothing
and a wrong secret get 401, and a standby's PostgreSQL log read across the
overlay from the node that placed it.

**And it found a fault as old as the pair: a node that stopped being
public had no way to say so.** The public node held the private one as
`public` at `192.168.1.55:443` — the LAN address it used to be named
after — so it aimed handshakes at somebody else's network, rewrote that
peer at every start while the kernel relearned the real address, and
would have offered a machine that can never answer as an edge for a
hostname. The protocol always had the fix and nothing used it:
`note_reachability` reads an **absent** endpoint as "an older node cannot
say" and an **empty** one as "I have none", and `report_for` never sent
the empty one. It does now, and a private address is refused on receipt —
the same judgement `ensure_self` makes about this node's own domain, which
is what heals a row that already holds one.

Two things about how it was found, both worth repeating. **A comparison
that returns a `bool` cannot say what it decided**: `tunnel::difference`
names the field and the caller logs it, and that one line turned "the
overlay changed again" into `field="endpoint"`. And **the first fix was
the wrong one** — the keepalive looked like the culprit, and teaching the
comparison to ignore it would have buried the real cause for good. What
survived that detour: a peer this node does not dial says **nought rather
than nothing**, because `configure_peer` writes what the peer carries and
an unset field asks the kernel to keep what it has.

Next: groups — health and failover across the upstreams of one name.
Today a dead replica keeps its share of the traffic until the owner
notices.

**The overlay is a mesh now, and the census is what made it one.** The
star was in the *rows*, not in the tunnel: `tunnel::peers` always built a
peer for every row with a key and an address, and a member only ever had a
row for its authority. `GET /api/network/roster`, collected in the same
pass as the errands, ends that — and the authority had the answer all
along, because every report already carried the reporting node's endpoint,
key, anchor and usage. Four rules hold it together, and each is a fault
avoided:

- **Reachability, never consent.** No `allows` on the wire: that column is
  what a node permits *the reader*, so the authority's copy says nothing
  about what a member permits a sibling — and a census carrying it would
  make `edge_candidates` offer nodes that never collect the errand. A
  learned member has an empty grant set, which is what keeps the control
  plane directed while the data plane meshes. No `usage` either (a
  measurement makes every census look changed) and no `kind` (derived from
  the endpoint, because a kind on the wire can arrive wrong).
- **An authority is healed, never described** — only a missing anchor.
  `collect` dials that endpoint every pass, and on a node with two
  authorities the census of one would otherwise move the address of the
  other.
- **An address two nodes claim is refused before the write**, and said out
  loud: `doctor` counts it, the attention card shows it. The unique index
  would have failed the write, and catching that into a warning is a peer
  that never works with nothing saying why.
- **Nothing is deleted**, and `AllowedIPs` is authorisation, so a stale key
  keeps the right to claim its `/32`. Pruning safely needs "every authority
  answered this pass" or a partition deletes peers.

**Two private nodes on the internet still have no path, and that is
accepted.** On one LAN they do: each node reports the addresses it holds
(`network::lan`, `getifaddrs` through `libc`, which compiles on the node
and on a laptop) and the peer is dialled at one — with **no keepalive**,
because the keepalive follows the NAT and its absence is what makes a wrong
guess silent. The refusals matter more than the rule: nothing inside
`10.42.0.0/16` is ever a candidate, because `wabot0` and every project
bridge live there and the naive version points each peer's tunnel at its
own address inside the tunnel.

**A name that is a private address is not an endpoint.** Naming a node
`192.168.1.55` — which used to be the only way into its console from
another machine on the LAN — made it *public*, with an endpoint its
authority held as dialable: offered as an edge for hostnames, sending
handshakes into its own network, and about to be handed to every member by
the census. The console answers on this machine's own addresses now, so the
workaround is not needed; `certs::own_names` carries them and
`wanted_names` deliberately does not, because a private address "resolves
here" through `ToSocketAddrs` and the DNS guard would not have stopped the
order.

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
- ~~**The overlay is a star, not a mesh.**~~ It was, and the reason is
  worth keeping: a join writes one row on each side, so two nodes enrolled
  by the same authority had never heard of each other and had no WireGuard
  session. Phase 7 did not notice because the verified topology always had
  the authority at one end; the standby dialling a primary on a third node
  was the case with no path, and the fix there was the peer travelling in
  the errand. **The star was never in the tunnel** — `tunnel::peers` builds
  a peer for every row that has a key and an address — so the answer was
  the census below, and the errand payloads stay as the thing that works
  before a node has collected one.

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
