# wabot-deploy

Container deployments on a node you own — the single-node counterpart
to wabot-cloud, with containerd instead of Kubernetes, an embedded
registry instead of Harbor, and SQLite instead of Postgres.

One binary. Two processes on the box: `containerd` and this one.

[`docs/architecture.md`](docs/architecture.md) has the design and the
reasoning behind it.

## Status

**M3** — `install` sets up the whole machine: it checks it can run the
node, installs containerd and crun, registers a systemd service and
starts it. What is left is deployments — the node can route a hostname
to a container but cannot yet start one.

| | |
| --- | --- |
| ✅ `install` | layout, config, database |
| ✅ `serve` | control plane, graceful shutdown |
| ✅ `doctor` | what is set up and what is not |
| ✅ edge | TLS, host routing, reverse proxy with upgrades, HTTP redirect |
| ✅ ACME | Let's Encrypt over HTTP-01, renewed in the background |
| ✅ console | a status page at `/`, server-rendered, no JavaScript |
| ✅ bootstrap | preflight, containerd + crun, the systemd unit, a service that starts |
| ⏳ next | deployments: containers this node starts, not just routes to |

`doctor` lists the install steps that have not shipped as
`not implemented yet` rather than hiding them.

## Running it

```sh
cargo build --release

# A node under /tmp on an unprivileged port — no root needed.
export WABOT_DEPLOY_DATA_DIR=/tmp/wd/data
export WABOT_DEPLOY_HTTPS_PORT=8443
export WABOT_DEPLOY_HTTP_PORT=8080

./target/release/wabot-deploy --config /tmp/wd/config.toml install
./target/release/wabot-deploy --config /tmp/wd/config.toml doctor
./target/release/wabot-deploy --config /tmp/wd/config.toml serve
```

```sh
curl -sk https://localhost:8443/healthz   # liveness: the process answers
curl -sk https://localhost:8443/readyz    # readiness: the database answers too

# Or without -k, trusting the CA that `install` exported:
curl -s --cacert /tmp/wd/data/certs/local-ca.crt https://localhost:8443/healthz

curl -si http://localhost:8080/  # 308 to HTTPS
```

Until a domain is configured the node serves a certificate from a local
authority, written to `<data_dir>/certs/local-ca.crt` — trust that once
and the warnings stop.

With `node.domain` set and port 80 reachable from the internet, the
node obtains a Let's Encrypt certificate and swaps it in without a
restart:

```sh
sudo wabot-deploy install --domain node.example.com --email you@example.com
```

`install` asks once and reports what happened; `serve` retries in the
background and renews at 30 days. A failure is never fatal — the local
certificate keeps serving, and `doctor` shows the reason.

Add `--acme-staging` while testing. Production refuses more than five
failed orders per account per hour, so debugging a DNS problem against
it locks you out for the rest of the hour; staging certificates are
untrusted by design, which is the trade.

With no routes configured every hostname reaches the control plane, so
a fresh node is usable at whatever address you can reach it on.

On a real node the defaults are the answer — `/etc/wabot-deploy/config.toml`,
`/var/lib/wabot-deploy`, port 443 — and `install` runs as root:

```sh
sudo wabot-deploy install --domain node.example.com
```

That checks the machine, installs containerd and crun, writes a systemd
unit, obtains a certificate and starts the service. It converges: run
it again and it does only what is missing.

| flag | |
| --- | --- |
| `--no-runtime` | leave containerd alone; something else manages it |
| `--no-system` | change nothing outside the data directory — for a container image, or a dry run |
| `--no-start` | install everything, start on the next boot |
| `--skip-preflight` | when you know something the checks do not |

The listeners bind `0.0.0.0` only on privileged ports — that is a real
node, where the edge terminating TLS is the point. On a high port,
which in practice means a developer, they bind loopback: there is no
authentication yet, and an unauthenticated console should not appear on
the network of whatever laptop it was started on.

## Configuration

`/etc/wabot-deploy/config.toml`, written by `install` and never
rewritten afterwards. Unknown keys are refused rather than ignored, so
a typo fails loudly instead of leaving a default quietly in place.

```toml
[node]
domain = "node.example.com"          # optional; without it, self-signed
data_dir = "/var/lib/wabot-deploy"

[edge]
https_port = 443
http_port = 80

[log]
filter = "info"
```

Environment overrides, for a container or a one-off run:
`WABOT_DEPLOY_DOMAIN`, `WABOT_DEPLOY_DATA_DIR`,
`WABOT_DEPLOY_HTTPS_PORT`, `WABOT_DEPLOY_HTTP_PORT`,
`WABOT_DEPLOY_LOG` (or `RUST_LOG`), `WABOT_DEPLOY_WORKERS`.

## Layout

| | |
| --- | --- |
| `src/main.rs` | CLI dispatch, tracing, the tokio runtime |
| `src/cli.rs` | the three verbs |
| `src/config.rs` | `config.toml` + environment overrides |
| `src/db.rs` | opening the database, applying migrations |
| `src/ledger.rs` | which install steps have run |
| `src/api.rs` | the control-plane HTTP surface |
| `src/edge/` | TLS, certificates, ACME, host routing, the reverse proxy |
| `src/console/` | the web console |
| `src/bootstrap/` | preflight, containerd + crun, the systemd unit |
| `assets/` | the design system, vendored — a node must not need a CDN |
| `src/commands/` | one module per verb |
| `migrations/` | embedded with `include_str!`, so the binary stands alone |

## Building

`scripts/build-linux.sh` produces the Linux binary from a Mac, via
Docker — the project builds natively on macOS too, but the thing you
deploy should be built for where it runs.

Needs a sibling checkout of the framework at
`../../framework/wabot-rust` — see `[patch.crates-io]` in
`Cargo.toml`. That goes away when `wabot 0.1` reaches crates.io.

## Your coding agent

`.claude/skills/` explains how to build with this framework — start it
at `wabot-rust-quickstart`. For Codex or another agent that reads a
skills directory:

```sh
scripts/install-skills.sh
```
