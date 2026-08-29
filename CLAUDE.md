# wabot-deploy

A single-node container platform: one binary that installs itself on a
Linux box, terminates TLS, receives images, runs containers and updates
itself. Read [`README.md`](README.md) for what it does and
[`docs/architecture.md`](docs/architecture.md) for why it is built this
way — §10 there is the record of what went wrong and what it taught.

**[`docs/architecture-rules.md`](docs/architecture-rules.md) governs how
new code is shaped.** Read it before writing any. This file is the rest:
the gate, the conventions, and how to work on a node.

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

## The architecture rules

Nine, written in full with their costs in
[`docs/architecture-rules.md`](docs/architecture-rules.md). They are
about **the shape of this software**. How the network of nodes is put
together is a separate subject with its own documents — see "The
network and the databases" below.

1. **One use case per file, named for who does what.** Five actors:
   `node_user_`, `tenant_user_`, `guest_`, `cli_user_`, `remote_node_`.
   Flat, with the actor in the filename, because a directory carrying
   the actor leaves the file called `create_project.rs`.
2. **A command has one body and two doors** — synchronous is calling
   `run`, asynchronous is a `#[command_handler]` calling the same `run`.
   **One synchronous caller means there is no command**: the body goes
   in the use case.
3. **Authorisation is per actor, and it is roles.** `authz::node_user`,
   `authz::tenant_user`. The **use case** checks whether the target is
   theirs; the command trusts.
4. **A command returns success or a reason, and nothing else.**
5. **Controllers are thin; a page is a file.** A controller reads the
   form, calls the use case, redirects. `src/pages/` renders.
6. **Zero comments** unless the comment explains *why it was done this
   way*. This supersedes the older habit of matching a file's comment
   density: new code carries none, and a file being refactored loses the
   ones that only restate it.
7. **Persistence answers about the data, never about who is asking.**
   The test is not "does it take an actor" but *could two callers
   legitimately disagree?* — an invariant belongs beside the `INSERT`,
   authorisation belongs in the use case.
8. **A button says it is working, and the page does not move.** Actions
   run over `fetch`, the button disables and reports, and nothing
   reloads or scrolls. **It never changes size** — the mark is drawn
   over the label, not after it — and an **icon-only button gets no
   mark at all**, because it has no label for one to replace.
   Enhancement only — see "The console works without JavaScript".
9. **A use case takes a request and hands back a response.** Both live
   in `src/dtos`, and the controller is what builds the request — so it
   arrives with its shape already right. The form's own spelling stops
   there: an unticked checkbox sends nothing, and a use case that knew
   `accept-host` would be one only a browser could call.

Where the tree stands: every use case's two ends are DTOs and no
controller draws — four guards read directories to keep it that way.
`src/views/pages/` holds 24 files; `console/services.rs` is still 6,800
lines of handlers and `console/nodes.rs` 3,700. No `cli_user_` or
`remote_node_` use case exists yet.

## The framework is ours

This is built on [wabot-rust](https://crates.io/crates/wabot), which we
own. When something is missing, the question is not "how do I work
around this" but **"is this a generic capability the framework lacks, or
is it wabot-deploy's own?"** If the first, it goes upstream and every
wabot app inherits it — that is how `#[raw]`, TLS, SQLite, cooperative
cancellation, `#[patch]`/`#[head]` and boosted form submission got
there.

The checkout lives at `../../framework/wabot-rust`. Changes there need
their own tests, a version bump and a publish before this repo can use
them; `Cargo.toml` depends on the published crate, not a path.

## Conventions the code already follows

**Tests state a claim, not the implementation.** Name them as sentences
(`a_stale_run_does_not_block_the_next_one`). A test that restates the
body of the function it tests passes forever and catches nothing — that
happened here once, and CI caught the drift. When a test guards against
a bug that shipped, say so in a doc comment on the test.

**A guard that names its inputs is right until somebody adds an input.**
It has cost this project five times. A scan reads a directory rather
than a list of files; a rule is expressed as a layering fact rather than
an allow-list. And a guard nobody has seen fail is not known to work —
reintroduce the fault, watch it go red, put it back.

**Verify against the node, not against the model in your head.** Almost
every hard bug in this project was invisible locally: the mount
namespace, the CNI teardown, the HTTP/2 downgrade, the staging
certificate that looked fine. If a change touches containerd, systemd,
CNI or ACME, it is not verified until it ran on a real node.

**And a test cannot see what one node knows about another.** In a test
both ends are one database and the rows are the ones the test wrote, so
nothing exercises the thing that has caused nearly every networked bug
here — *derived state that nothing recomputes when its input arrives
over the network*.

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
scripting off. Rule 8 is enhancement on top of that, never instead of
it.

**The framework's client runtime is already there.** `/_wabot/client.js`
comes with the `ui-hypertext` feature, every page loads it, and it
**boosts both links and form submissions**: a click swaps the view with
`innerHTML` instead of loading a page, and a submit posts over `fetch`
and swaps the result without scrolling. Two consequences, both of which
shipped as bugs before anyone noticed:

- A `<script>` inside swapped HTML **never runs**. Inline scripts in a
  view are dead on arrival for anyone who clicked a link.
- A listener attached once at load belongs to a form the next swap
  discards.

So client behaviour goes through `wabot.island(id, mount)` and a
`<wabot-island>` host, which the runtime re-hydrates after every swap
and tears down when the host leaves. `assets/console.js` registers six:
`fields`, `copy`, `project-live`, `updates-live`, `node-live` and
`logs-live`. Prefer declarative — the markup carries `data-when` /
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
measured. Page CSS — the shell, the meter, the cards, the button states
rule 8 defines, anything this console invented — stays in
`src/console/layout.rs`. The split is the thing, not the file's
provenance.

Two tests read the stylesheet directly rather than a copy of it:
`the_two_dark_palettes_agree`, because dark is written twice and CSS
cannot make a media query set an attribute, and
`a_classed_submit_is_not_the_primary_button`, because
`button[type='submit']` outranks every `.btn-*` variant and takes them
all with it.

**And anything visual has to be looked at by somebody.** Without a
session the console answers 302, so four UI faults in one afternoon — a
repeated word, a hard-to-hit control, a misaligned icon, a password
manager hijacking a field — were all found by Jorge and none by the
tests. Never fabricate a session or a token in the node's database to
test a page; ask for the click. (`wabot-deploy passwd <username>` exists
for the operator locked out of their own node — that is recovery, not a
way to conjure a session.)

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

## Releasing

```sh
# bump version in Cargo.toml, commit, then:
git tag -a vX.Y.Z -m "…" && git push origin main vX.Y.Z
```

The tag is the trigger and the tag is the version; the workflow refuses
to build if `Cargo.toml` disagrees. It produces one static musl binary
plus its checksum, which is what the console's updater downloads.

Publishing a release is outward-facing. Ask first.

## The network and the databases

A separate subject, with its own documents and its own decisions:

- [`docs/network.md`](docs/network.md) — the plan and the record for
  enrolment, the overlay, errands, capabilities, edges, the census and
  node management. **Read its "What it is for" before the phases.**
- [`docs/databases.md`](docs/databases.md) — managed PostgreSQL on top
  of `service` and `replica`.
- [`docs/naming.md`](docs/naming.md) — how a name is derived and which
  authority can sign it.

Four things belong here because they change what you may do, not what
happened:

- **A node's id is its own**, minted at `install` and kept for ever.
  The enrolling node allocates the overlay address and nothing else.
- **A node that joined before v0.4.0 must re-join.** The direction
  inverted and a hash cannot be presented.
- **The first hop of an update is manual**, with `cross.sh`: a node too
  old to understand the instruction refuses it with a sentence about
  versions, and that is exactly the node somebody wants to update.
- **A service is administered from the node where it was created.** What
  lands on a receiving node is derived: not editable there, evictable
  there. A **replica** is the unit, not a service.

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
- `scripts/deploy.sh` prints `line 76: release: command not found` on
  every run. Harmless, and it is exactly where somebody looks when
  something is wrong.
- `doctor` prints the overlay port from the config under a comment
  promising what the kernel says. The peers below it are read from the
  kernel; the port is not.
- Health and failover across the upstreams of one name. Today a dead
  replica keeps its share of the traffic until the owner notices.

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
