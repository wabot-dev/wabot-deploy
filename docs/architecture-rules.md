# The rules

Written down as they are decided, in the order they were decided, with
what each one costs. A rule with no cost written beside it is one
somebody will break for a good reason and not notice they did.

## 1. One use case per file, named for who does what

**The most important one.** A file holds exactly one use case, and its
name says which role performs it and what it does:

```
tenant_user_create_project.rs
tenant_user_update_service_replica.rs
node_user_update_service_replica.rs
cli_user_install_node.rs
remote_node_host_service.rs
```

Shared work lives in **commands** — `UpdateServiceReplica` takes its
parameters and does the job — and a command may be run synchronously or
queued, whichever suits the caller.

### The four actors

Everything this node does is done *by somebody*, and there are four:

| Actor | Reaches the node through | What it looks like today |
|---|---|---|
| **Node user** | the machine's console | `console::nodes`, `console::people`, `console::updates` |
| **Tenant user** | an organisation's console | `console::tenant` |
| **CLI user** | a terminal on the machine | `commands::install`, `doctor`, `join`, `passwd`, `backup` |
| **Remote node** | errands and the control-plane API | `network::collect::carry_out`, `network::api` |

**The CLI subcommands are use cases too**, and treating them as a
separate kind of thing was the mistake in the first draft of this rule.
`wabot-deploy join` and the console's join form are one use case with two
actors; `install` is a use case with exactly one. Naming the actor is
what makes that visible instead of arguable.

The fourth is the one that is easy to forget and the one this codebase
has paid for most: a remote node asking for something is not a special
case of a person asking, and every fault in `docs/network.md`'s phase 4-7
list came from code that had not decided which of the two it was serving.

### Flat, with the actor in the filename

`src/usecases/` holds them all, unnested. The actor is a prefix rather
than a directory, for two reasons:

- **A filename travels.** It is what a stack trace prints, what a grep
  returns, what a review shows. A directory that carries the actor
  leaves the filename saying `create_project.rs`, which is the thing this
  rule exists to stop.
- **Sorting groups them anyway.** `cli_user_*`, `node_user_*`,
  `remote_node_*`, `tenant_user_*` — the listing is the grouping, without
  a path that repeats what the name already says.

### What this fixes

The files it replaces are 9000 and 6000 lines. Nobody reads one whole,
which is how the same rule ends up written twice without anybody
noticing: the placement spread was enforced in two places and neither
could be shown to matter, and a guard that matched one call shape passed
while the other half went untranslated.

It also settles a question that has cost time twice: *whose page is
this*. `console::people` grew a scope selector, a cross-frontier check
and five `administers_scope` calls before anybody read the words on the
page and found they were wrong. With the actor in the filename there is
nowhere for that to hide.

### What it costs

Many small files, and a reader who wants "everything about services" has
to open a directory rather than a file. That is the trade taken
deliberately: a directory listing is a table of contents, and a
9000-line file is not.

## 2. A command has one body and two doors

`UpdateServiceReplica` is a plain type with an inherent
`run(...) -> Result<T>`. **Synchronous is calling it. Asynchronous is a
`#[command_handler]` whose `handle` calls that same `run`.**

The framework offers only the second half: `wabot::async_jobs::run_command`
enqueues and spawns, and nothing there runs a command and hands back its
outcome. Rather than ask upstream for a synchronous executor — which is
a version, a publish and a wait — the command carries its own body and
the queue is one caller of it.

It is also what the deploy path already does, where
`DeployHandler::handle` calls `Deployer::deploy`. This rule names the
shape rather than inventing it.

### What it costs

Two ways to reach the same work, and a use case has to choose. The
choice is not free: a synchronous call can tell somebody *why* it was
refused in the same POST, and a queued one can only say it was accepted.
That is the whole reason both doors exist — see the resources form,
which refuses with numbers, against a placement, which cannot.
