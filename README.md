# wabot-deploy

Built with [wabot-rust](https://github.com/wabot-dev/wabot-rust).

```sh
cp .env.example .env   # already done — fill in the keys
cargo run
```

```sh
curl -s localhost:3000/api/notes/ | jq
curl -s -H 'content-type: application/json' -d '{"text":"hello"}' \
     localhost:3000/api/notes/ | jq
```

Postgres is expected at `DATABASE_URL`. The framework creates the
tables it owns on first use; tables of your own belong in a migration
(`wabot-migrate`).

## Layout

| | |
| --- | --- |
| `src/main.rs` | wiring and the lifecycle |
| `src/api.rs` | the HTTP surface |

## Your coding agent

`.claude/skills/` explains how to build with this framework — start it
at `wabot-rust-quickstart`. For Codex or another agent that reads a
skills directory:

```sh
scripts/install-skills.sh
```
