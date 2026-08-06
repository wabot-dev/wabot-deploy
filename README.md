# wabot-deploy

Container deployments on a node you own — the single-node counterpart
to wabot-cloud, with containerd instead of Kubernetes, an embedded
registry instead of Harbor, and SQLite instead of Postgres.

One binary. Two processes on the box: `containerd` and this one.

[`docs/architecture.md`](docs/architecture.md) has the design and the
reasoning behind it.

## Status

**M0** — the CLI, configuration, the database and the health
endpoints. The node installs, reports its state and serves; it does
not deploy anything yet.

| | |
| --- | --- |
| ✅ `install` | layout, config, database |
| ✅ `serve` | control plane, graceful shutdown |
| ✅ `doctor` | what is set up and what is not |
| ⏳ M1 | the edge: TLS on 443, host routing, reverse proxy |
| ⏳ M2 | ACME |
| ⏳ M3 | containerd + crun, the systemd unit, the rest of the bootstrap |

`doctor` lists the install steps that have not shipped as
`not implemented yet` rather than hiding them.

## Running it

```sh
cargo build --release

# A node under /tmp on an unprivileged port — no root needed.
export WABOT_DEPLOY_DATA_DIR=/tmp/wd/data
export WABOT_DEPLOY_HTTPS_PORT=3000

./target/release/wabot-deploy --config /tmp/wd/config.toml install
./target/release/wabot-deploy --config /tmp/wd/config.toml doctor
./target/release/wabot-deploy --config /tmp/wd/config.toml serve
```

```sh
curl -s localhost:3000/healthz   # liveness: the process answers
curl -s localhost:3000/readyz    # readiness: the database answers too
```

On a real node the defaults are the answer — `/etc/wabot-deploy/config.toml`,
`/var/lib/wabot-deploy`, port 443 — and `install` runs as root.

The control plane binds **127.0.0.1** until the edge lands. There is no
authentication yet, and an unauthenticated API on `0.0.0.0` is not a
default worth having for one milestone's convenience.

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
| `src/commands/` | one module per verb |
| `migrations/` | embedded with `include_str!`, so the binary stands alone |

## Building

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
