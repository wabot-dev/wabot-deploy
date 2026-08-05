---
name: wabot-rust-framework
description: Use when starting work in the wabot-rust workspace, adding a crate, wiring dependency injection, or translating an API from wabot-ts. Covers the crate layout (wabot-core / wabot-feature-* / wabot-addon-* / the wabot umbrella), the proc-macro decorators and what each expands to, the DI container and its three lifetimes (register_instance, register_singletons!, register_transients!) including the rule that a controller reading Auth must be transient, the DynX newtype shim for trait objects as container keys, #[derive(Validate)] and ModelInfo as the single source of truth consumed by LLM tool schemas and column binding alike, the feature flags that keep heavyweight addons opt-in, and the systematic differences from the TypeScript original — no import-time registration, no metadata stores where a trait suffices, and errors as enums rather than a CustomError class.
---

# wabot-rust: the map

A Rust port of [`wabot-ts`](../../wabot-ts). Read the two side by side —
file names mirror each other on purpose.

## Layout

| Kind | Crate | Mirrors |
| --- | --- | --- |
| foundation | `wabot-core` | `src/core/*` |
| macros | `wabot-macros` | the decorators |
| feature | `wabot-feature-<area>` | `src/feature/<area>` |
| addon | `wabot-addon-<area>-<impl>` | `src/addon/<area>/<impl>` |
| umbrella | `wabot` | the published package |

One crate per `feature/*` and `addon/*` directory. A new crate goes in
the workspace `members` list **and** `[workspace.dependencies]`.

## Registration is explicit, and that is the biggest difference

wabot-ts registers controllers as an **import-time side effect** of
decorators, then discovers them by walking the filesystem. Rust has no
dynamic import, so nothing is discovered: you call
`Controller::register_routes(&container, router)` and
`register_singletons!(&container, Foo, Bar)`.

The payoff is that a forgotten registration is a **compile error**
rather than a controller that silently never mounts. Do not try to
recreate discovery — `ProjectRunner` deliberately does not have it.

## The container, and its one trap

```rust
let c = Container::new();
c.register_instance::<Config>(Arc::new(config));  // a value you built
register_singletons!(&c, ChatBot, Rooms);          // built once, shared
register_transients!(&c, UserController);          // built per resolve
```

**A controller that reads `Auth` must be transient.** `Auth` lives in
the request's child container; a singleton controller is built once, so
its `Arc<Auth>` would be the first caller's identity served to
everyone. `guarded_controller.rs` has a 24-way concurrent test on this.

A transient is built with the **resolving** container (tsyringe's
behaviour); a singleton keeps the container it was built with, because
it is cached and shared, and capturing one request's container inside
one would leak that request into every later call.

A trait object cannot be a container key. The convention is a newtype
shim: `DynLocker(pub Arc<dyn Locker>)`, plus `register_locker(&c, …)`
and `locker(&c)` helpers.

## `#[derive(Validate)]` is the single source of truth

It bakes a `static ModelInfo` into the type. Three consumers read it:
the validator, the **LLM tool schema** a mindset ships to a provider,
and the **column binding** in the Postgres columnar store. Don't add a
second way to describe a type — the one the compiler doesn't read is
the one that rots.

Field *types* are inferred from Rust types (`Option<T>` → optional,
`Vec<T>` → array); only semantic constraints need attributes
(`#[min(1)]`, `#[is_not_empty]`, `#[is_in("a","b")]`,
`#[description("…")]`).

## Macros

`#[injectable]` `#[singleton]` decorate a struct. Everything else
decorates an **impl block** and interprets inner attributes:
`#[rest_controller]` (`#[get]`/`#[post]`/`#[middleware]`),
`#[chat_controller]` (`#[cmd]`/`#[telegram]`/`#[socket]`),
`#[socket_controller]` (`#[on_socket_event]`), `#[ui_controller]`
(`#[view]`/`#[action]`), `#[tools]` (`#[tool]`).

Macros emitting `wabot-core` paths must go through
`crate::util::wabot_core_path()` so they work whether the user pulls the
umbrella or core directly.

## Features

Heavyweight addons are **not** default: telegram, socket, the LLM
adapters, TSX rendering. An app that doesn't need a channel should not
pay for its dependency tree. `wabot::testing` is off by default too, so
a mock LLM adapter can't reach production by accident.

## What is deliberately absent

- **No `CustomError`.** Every crate has a `thiserror` enum;
  `RestError` even carries the HTTP code.
- **No metadata stores where a trait suffices.** TS needs runtime
  registries because it cannot express "these methods, implemented
  elsewhere"; Rust traits can.
- **No `inventory`-based auto-registration.** It would force every
  consumer crate to add `inventory` to its own Cargo.toml.

`CLAUDE.md` at the workspace root carries the full phase-by-phase
notes, including the mistakes and why each design went the way it did.
Read the section for the area you are touching before changing it.
