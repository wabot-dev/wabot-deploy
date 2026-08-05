---
name: wabot-rust-async
description: Use when running work in the background in wabot-rust — commands, job handlers, cron schedules, retries, deduplication, distributed locking — or when composing an application's subsystems and shutting them down. Covers #[command] / #[command_handler] / #[cron_command] / #[cron_handler] and their options, six-field cron expressions, run_command vs schedule_command, the retry semantics ported quirks-and-all from wabot-ts, dedup at enqueue under a lock, the Locker trait with its in-process and Postgres advisory-lock implementations, how a job inherits the dispatcher's audit actor and correlation id, the three-phase ShutdownManager and why the order matters, install_panic_reporter and how it differs from Node crash handlers, and ProjectRunner — what it composes and why it is deliberately not a filesystem scanner.
---

# Background work and lifecycle

## Commands and handlers

```rust
#[command("send-email")]
#[derive(Serialize, Deserialize, Validate)]
struct SendEmail { to: String }

#[injectable]
#[derive(Default)]
struct SendEmailHandler;

#[command_handler(SendEmail, retry_delays = [5, 30], dedup = 300)]
impl SendEmailHandler {
    async fn handle(&self, data: SendEmail) -> Result<(), AsyncError> { … }
}

run_command(&container, &SendEmail { to: "a@b.c".into() }).await?;
schedule_command(&container, &cmd, ScheduleAt::in_minutes(5)).await?;
run_async_workers(container, vec![SendEmailHandler::__handler_entry(&c)], crons).await?;
```

Enqueue is **free functions taking `&Container`**, not an `Async`
singleton: the TS class holds the repo, scheduler and locker, and the
Rust equivalent would have to hold a container that holds it.

`run_command` = enqueue **plus** best-effort immediate execution. If
this process doesn't handle the command, the job waits for a tick —
here or on another node.

## Cron

Six fields, seconds first: TS's `'0 2 * * *'` becomes `"0 0 2 * * *"`.
Five-field strings parse but mean something else.

A cron tick queues an ordinary job under the cron's command name, so
the runner needs a `CommandRegistry` entry for it too —
`cron_command_entry` is the bridge, and without it every cron job dies
with `CommandNotRegistered`.

## Retries, ported quirks and all

`set_as_started` increments `intent_number`, and `set_as_failed`
increments it again when scheduling a retry, so the retry-delay index
advances by **two** per attempt. That is what wabot-ts does. Don't fix
it in Rust alone or the two stacks diverge on the same stored row.

Dedup is enforced **at enqueue**, under a lock, keyed by SHA-256 over
canonical JSON so key order and nulls don't perturb it.

## Locking

`Locker` is a core trait: `InProcessLocker` for one process,
`PgLocker` (advisory locks) for several. `advisory_lock_key` is
**protocol, not implementation** — it reproduces TS's derivation
exactly, so a Rust node and a TS node locking the same logical key land
on the same bigint.

An advisory lock holds a pooled connection for the critical section.
Size for that.

## Jobs inherit who asked

`JobData` carries `actor` + `request_id`, captured at enqueue and
restored around the handler. A command is **provenance**, never the
actor: the runner sets `source = command:<name>` and leaves the actor
as whoever dispatched it. A cron tick has no human behind it, so the
schedule itself is the actor.

## Shutdown

```rust
shutdown.register(ShutdownTask::new("jobs", ShutdownPhase::Drain, move || …));
```

Three ordered phases — **intake** (stop accepting), **drain** (finish
what is running), **close** (release resources). Draining before intake
closes is pointless because new work keeps arriving; closing the pool
before draining fails the jobs the drain was meant to finish. Tasks
within a phase run concurrently.

A timeout **names the tasks still running** and yields exit code 1, so
an orchestrator can tell a stuck drain from a clean one. A task that
panics doesn't stop the rest.

`install_panic_reporter()` reports panics through `tracing`. It does
**not** exit: tokio contains a panicking task, and killing a server
because one request panicked is a worse default. Set `panic = "abort"`
if you disagree.

## ProjectRunner

```rust
ProjectRunner::new(container)
    .service("rest", run_rest_controllers(router, cfg))
    .service("jobs", run_async_workers(c, commands, crons))
    .on_shutdown(ShutdownTask::new("pool", ShutdownPhase::Close, …))
    .run().await
```

It is **not** a scanner — half of TS's walks the source tree to trigger
decorator side effects, which Rust neither can nor should do. What it
buys: one signal handler and one drain order, a service that ends takes
the process with it (a dead REST server must not leave a process
looking healthy), and an exit code that tells those apart.
