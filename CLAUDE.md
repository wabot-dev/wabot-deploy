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

**Errors are values somebody can act on.** A failure that only reaches
the journal is a failure nobody sees: put the reason where the person
looking for it will be (the run row, the service row, `doctor`, the
page).

**No JavaScript in the console** beyond the one `EventSource` on the
node page, and that page renders complete without it.

## Design

The console follows the Wabot design system: no borders, no shadows, no
hover state changes, primary actions are black, brand orange is for
highlights only, sentence case, no emoji as iconography. Status is a
coloured dot plus a word. `src/console/layout.rs` holds the page-level
CSS; the tokens come from `assets/`, vendored so a node never needs a
CDN.

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
