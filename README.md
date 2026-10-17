# wabot-deploy

Container deployments on a node you own — the single-node counterpart to
wabot-cloud, with containerd instead of Kubernetes, an embedded registry
instead of Harbor, and SQLite instead of Postgres.

One binary. Two processes on the box: `containerd` and this one.

[`docs/architecture.md`](docs/architecture.md) has the design and the
reasoning behind it; [`docs/roadmap.md`](docs/roadmap.md) is what is
missing before this belongs in front of real traffic, and in what order.

## Install

```sh
v=0.23.0  # https://github.com/wabot-dev/wabot-deploy/releases
base=https://github.com/wabot-dev/wabot-deploy/releases/download/v$v

curl -fsSLO $base/wabot-deploy-$v-x86_64-linux
curl -fsSL  $base/wabot-deploy-$v-x86_64-linux.sha256 | sha256sum -c -

chmod +x wabot-deploy-$v-x86_64-linux
sudo ./wabot-deploy-$v-x86_64-linux install --domain node.example.com --email you@example.com
```

One static binary for x86_64 Linux: SQLite is compiled in, TLS is
rustls, and there is no libc to match. You install by hand once; after
that the console updates the node.

`install` checks the machine, installs containerd, crun and the CNI
plugins, registers a service, obtains a certificate, starts it and prints
the setup token the console asks for. It converges: run it again and it
does only what is missing.

**systemd and OpenRC** are both understood — Debian, Ubuntu and the rest
on one side, Alpine on the other. On Alpine it also does what a systemd
distribution had already done for you: enables the `cgroups` service,
loads `overlay`, and installs `iptables` and `iproute2`, which the
container network is built with. On a machine with neither manager the install still
writes everything and tells you that starting the node is yours to
arrange.

The certificate comes last, after the node is running: the HTTP-01
challenge is answered on :80 by the node itself, so it has to be up
first. If it does not arrive, the install **fails** — an install that
reports success while serving a certificate no browser trusts is a
failure discovered later, in a browser, by somebody who was not there.
The node keeps running and retrying either way, so the console is
reachable to fix the domain from.

Pass `--self-signed` when this node will never have a public certificate
— a machine on a private network. That turns the asking off rather than
letting it fail twice a day, and the console still serves HTTPS on the
node's own authority, which covers **its addresses** as well as its names:
`https://192.168.1.10` opens it from another machine on the same network.
A name whose DNS is merely still propagating needs no flag; the install
says what is missing and the node keeps asking.

| flag | |
| --- | --- |
| `--domain` | the hostname this node answers to |
| `--email` | contact for the certificate authority — it warns you before expiry |
| `--acme-staging` | Let's Encrypt staging while testing; production refuses more than five failed orders an hour |
| `--self-signed` | never ask a public authority; serve this node's own certificate (the node page turns it back on) |
| `--no-runtime` | leave containerd alone; something else manages it |
| `--no-system` | change nothing outside the data directory |
| `--no-start` | install everything, start on the next boot |
| `--skip-preflight` | when you know something the checks do not |

Then open the console at your domain, paste the setup token, and create
the first administrator.

## What it does

| | |
| --- | --- |
| **Projects and services** | a service is an image, an environment and the ports it declares |
| **Ports** | each one says whether it is published on the node's IP, served over HTTPS on a hostname, both, or neither |
| **Certificates** | Let's Encrypt over HTTP-01, one per hostname, renewed in the background |
| **Registry** | `docker push` / `ctr push` straight into containerd's content store — no second copy of any layer |
| **Releases** | every push is a release, pinned by digest; roll back to an earlier one from the console |
| **Config history** | environment changes are versioned separately, so reverting one does not move the image |
| **People** | admin/member on the node, owner/deployer/viewer per project, invitations by link |
| **Node page** | memory attributed to the platform, the runtime, the shims and your containers |
| **Updates** | install a published release with one click; never on a timer |
| **A second node** | a node with a public address can enrol another one, which then takes instructions from it |

## A second node

From the nodes page of a node that answers to a name: add a private
node, and it mints a join token, shown once.

Take it to the other machine — installed already, this is not an
installer — and spend it either way round: paste it into that node's own
nodes page, or run `wabot-deploy join <token>` there. Both record the
first node as an authority, tell it so, and take an address on the
overlay.

Nothing is sent across yet. Authority is directed and revocable from the
node that granted it, which is the whole model:
[`docs/network.md`](docs/network.md) is the plan and the record of what
was decided.

## Updating

The console lists what has been published, renders each release's notes,
and installs one when you ask. It verifies the published checksum, runs
the downloaded binary to confirm it is the version it claims, copies the
database before anything can migrate it, swaps the binary atomically and
restarts.

Nothing happens on its own. A node that updates itself is a node that
restarts everything on it at a moment nobody chose.

A release that changes the systemd unit says so in its notes — the
updater does not rewrite it. Re-run `install` for that.

## Configuration

`/etc/wabot-deploy/config.toml`, written by `install` and never rewritten
afterwards — except `[backup.s3]`, which the console's Backup tab writes
there on purpose. Unknown keys are refused rather than ignored, so a typo
fails loudly instead of leaving a default quietly in place.

```toml
[node]
data_dir = "/var/lib/wabot-deploy"

[edge]
https_port = 443
http_port = 80

[acme]
directory = "production"             # or "staging", or a directory URL
email = "you@example.com"

[overlay]
port = 51820

[log]
filter = "info"
```

**What the console changes is not here.** The node's domain and whether it
asks a public authority for certificates live in the database, and the node
page changes both. `install --domain` and `install --self-signed` write
them there too. A file may still carry `node.domain` or `acme.disabled` —
every install before this wrote them — and they are read **once**, at the
first start, and never again; `doctor` says so when it finds them.

Environment overrides, for a container or a one-off run:
`WABOT_DEPLOY_DOMAIN`, `WABOT_DEPLOY_DATA_DIR`, `WABOT_DEPLOY_HTTPS_PORT`,
`WABOT_DEPLOY_HTTP_PORT`, `WABOT_DEPLOY_LOG` (or `RUST_LOG`),
`WABOT_DEPLOY_WORKERS`.

## Running it locally

```sh
cargo build --release

# A node under /tmp on unprivileged ports — no root, no containerd.
export WABOT_DEPLOY_DATA_DIR=/tmp/wd/data
export WABOT_DEPLOY_HTTPS_PORT=8443
export WABOT_DEPLOY_HTTP_PORT=8080

./target/release/wabot-deploy --config /tmp/wd/config.toml install --no-system --skip-preflight
./target/release/wabot-deploy --config /tmp/wd/config.toml serve
```

```sh
curl -sk https://localhost:8443/healthz   # liveness: the process answers
curl -sk https://localhost:8443/readyz    # readiness: the database answers too

# Or without -k, trusting the CA that `install` exported:
curl -s --cacert /tmp/wd/data/certs/local-ca.crt https://localhost:8443/healthz
```

Without a domain the node serves a certificate from a local authority,
written to `<data_dir>/certs/local-ca.crt` — trust that once and the
warnings stop.

The listeners bind `0.0.0.0` only on privileged ports — a real node,
where the edge terminating TLS is the point. On a high port, which in
practice means a developer's laptop, they bind loopback.

`wabot-deploy doctor` reports what is configured, what is installed and
what is missing. Read-only; safe on a live node.

## Layout

Six rooms, and dependencies point inward — `domain` ← `infra` ←
`shell` ← `features` ← `web` ← `cli` — each importing only what is
beneath it. See
[rules 11 and 12](docs/architecture-rules.md).

| | |
| --- | --- |
| `src/features/` | one folder per feature — `access`, `backups`, `databases`, `network`, `node`, `organisations`, `people`, `projects`, `services`, `updates` — each holding its use cases, commands, requests, responses, controllers, pages and words |
| `src/domain/` | what the business is: entities, values, the pure policies, who may do what, errors |
| `src/infra/` | everything that talks to something: |
| &nbsp;&nbsp;`repositories/`, `accounts/`, `db.rs`, `ledger.rs`, `config.rs` | the rows, the accounts and sessions, migrations, the install ledger, configuration |
| &nbsp;&nbsp;`deploy/` | from a row to a running container: placement, routing, DNS checks, quotas |
| &nbsp;&nbsp;`runtime/`, `bootstrap/` | the containerd client; preflight, containerd + crun + CNI, the systemd unit |
| &nbsp;&nbsp;`edge/`, `registry/` | TLS, certificates, ACME, host routing, the reverse proxy; the OCI store over containerd's content |
| &nbsp;&nbsp;`network/`, `node/`, `update/`, `backup/` | the nodes it knows and the errands between them; this machine; the self-update; taking and restoring backups |
| `src/shell/` | what every feature's doors are built with: the console's state, the session and form readers, the layout, the pieces and words more than one feature uses |
| `src/web/` | the router that mounts every feature, and the doors no feature owns: health, the control plane, the registry, the page a secret is shown on once |
| `src/cli/` | the verbs: `install`, `serve`, `doctor`, `join`, `setup-token`, `passwd`, `backup`, `restore`, `restore-node`, `clean`, `log-writer`, `containerd` |
| `migrations/` | embedded with `include_str!`, so the binary stands alone |
| `assets/` | the design system, vendored — a node must not need a CDN |

## Building

```sh
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --all --check
```

Needs `protoc` at build time: `containerd-client` generates its gRPC
bindings in a build script.

Two profiles, and the second exists by experience. `release` is full LTO
with one codegen unit — the binary lives for months on a machine, so
minutes of link time are traded against every request it will serve.
`node` drops to thin LTO and four units, because a release build on a
one-core VM takes the machine down with it.

| | |
| --- | --- |
| `scripts/deploy.sh user@host [install flags…]` | builds **on** the node over SSH and installs there |
| `scripts/build-linux.sh [arch]` | builds the Linux binary from a Mac via Docker — slow on Apple Silicon |

Releases are cut by tag: pushing `v1.2.3` builds a static musl binary and
publishes it, after checking that the tag and `Cargo.toml` agree.

## Your coding agent

[`CLAUDE.md`](CLAUDE.md) is the short version: how to build, test and
deploy this, and the conventions the code already follows.

`.claude/skills/` explains how to build with the wabot framework — start
at `wabot-rust-quickstart`. For Codex or another agent that reads a
skills directory:

```sh
scripts/install-skills.sh
```
