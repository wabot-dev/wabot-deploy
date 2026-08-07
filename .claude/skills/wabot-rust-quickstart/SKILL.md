---
name: wabot-rust-quickstart
description: Use when building a new application with wabot-rust, or when you need a complete, correct starting point rather than an explanation — the Cargo.toml with the right feature flags, a main.rs that boots, a REST controller with validated requests and a guard, a chat bot with a mindset and tools, background jobs, and the ProjectRunner that composes them under one shutdown. Every snippet here is taken from code that compiled; start by copying, then read the area skill (wabot-rust-controllers, wabot-rust-llm, wabot-rust-async, wabot-rust-persistence) for the reasoning and the traps.
---

# Building an application

This is the shortest path from nothing to a running service. The
`examples/` tree was removed, so this file is the reference — the code
below is lifted from applications that compiled, not written from
memory. When something here disagrees with the crate docs, the crate
docs are right (`cargo doc --open`).

## 1. `Cargo.toml`

Features decide what you pay for. `rest` and `chat` are cheap; the LLM
adapters, telegram and sockets each pull a large tree.

```toml
[package]
name = "my-app"
version = "0.1.0"
edition = "2021"

[dependencies]
wabot = { version = "0.2", features = ["rest", "chatbot", "addon-anthropic"] }
tokio = { version = "1", features = ["full"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
async-trait = "0.1"
parking_lot = "0.12"
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }

[dev-dependencies]
# Harnesses are off by default so a mock LLM adapter can't reach production.
wabot = { version = "0.2", features = ["testing"] }
```

| Feature | Pulls in |
| --- | --- |
| `rest` | REST controllers (axum) |
| `chat` | chat controllers + `chatbot` |
| `chatbot` | the ChatBot / mindset stack |
| `ui` | server-rendered pages, islands |
| `async-jobs` | commands, cron, the job runner |
| `pg` | Postgres stores, locker, migrations |
| `sqlite` | SQLite stores and migrations — one writer, WAL, `bundled` |
| `addon-async-sqlite` | job and cron repositories on SQLite |
| `rest-tls` | rustls on the REST server: static config or a dynamic SNI resolver |
| `ui-hypertext` | `rsx!` pages — validates element *and* attribute names at compile time |
| `addon-anthropic`, `addon-openai`, `addon-openrouter`, … | one LLM provider each |
| `addon-telegram`, `addon-socket` | one channel each |
| `testing` | the harnesses — **dev-dependencies only** |

## 2. A REST service, complete

```rust
use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};

use wabot::prelude::*;
use wabot::rest::axum::http::request::Parts;
use wabot::rest::{run_rest_controllers, RestError, RestResult, RestServerConfig};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    pub id: String,
    pub name: String,
}

// A service: built once, shared.
#[singleton]
#[derive(Default)]
pub struct UserService {
    store: RwLock<HashMap<String, User>>,
}

impl UserService {
    fn get(&self, id: &str) -> Option<User> {
        self.store.read().get(id).cloned()
    }
    fn create(&self, name: String) -> User {
        let id = format!("u-{}", self.store.read().len() + 1);
        let user = User { id: id.clone(), name };
        self.store.write().insert(id, user.clone());
        user
    }
}

// A request: path, query and body are merged into this one struct and
// validated before the handler runs.
#[derive(Debug, Deserialize, Validate)]
pub struct GetUser {
    #[description("user id (the `:id` path param)")]
    #[is_not_empty]
    pub id: String,
}

#[derive(Debug, Deserialize, Validate)]
pub struct CreateUser {
    #[description("display name")]
    #[is_not_empty]
    #[min_length(2)]
    #[max_length(40)]
    pub name: String,
}

// A guard. Middleware is rejection-only: it can refuse, not decorate.
#[singleton]
#[derive(Default)]
pub struct ApiKeyMiddleware;

#[async_trait]
impl Middleware for ApiKeyMiddleware {
    async fn handle(&self, parts: &Parts, _container: &Container) -> RestResult<()> {
        match parts.headers.get("x-api-key").and_then(|v| v.to_str().ok()) {
            Some("secret") => Ok(()),
            _ => Err(RestError::Unauthorized("missing or bad x-api-key".into())),
        }
    }
}

#[singleton]
pub struct UserController {
    service: Arc<UserService>,
}

#[rest_controller("/api/users")]
impl UserController {
    #[get("/:id")]
    async fn get_by_id(&self, req: GetUser) -> RestResult<User> {
        self.service.get(&req.id).ok_or_else(|| RestError::NotFound(req.id))
    }

    #[post("/")]
    #[middleware(ApiKeyMiddleware)]
    async fn create(&self, req: CreateUser) -> RestResult<User> {
        Ok(self.service.create(req.name))
    }
}

#[tokio::main]
async fn main() -> std::io::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,wabot=debug".into()),
        )
        .init();

    let c = Container::new();
    register_singletons!(&c, UserService, ApiKeyMiddleware, UserController);

    let router = UserController::register_routes(&c, wabot::rest::axum::Router::new());
    run_rest_controllers(router, RestServerConfig::from_env()).await
}
```

Four things to notice, each of which is a mistake if you get it wrong:

- **`register_singletons!` is not optional.** Nothing is discovered;
  a type you forget to register panics on resolve.
- **A controller that reads `Auth` must be `register_transients!`
  instead.** `Auth` lives in the request's child container, and a
  singleton controller would serve the first caller's identity to
  everyone.
- **`register_routes` returns the router** — chain it per controller.
- **`RestServerConfig::from_env()`** reads `PORT` / `BIND_ADDR`.

## 3. A chat bot with a mindset and tools

```rust
use wabot::prelude::*;                 // ChatBot, ChatMessage, Container, macros
use wabot::mindset::{
    Mindset, MindsetDescription, MindsetIdentity, MindsetModelRef, MindsetModels,
    MindsetOperator, ModelKind,
};

#[derive(Debug, Deserialize, Validate)]
struct ReadOrder {
    #[description("The order id")]
    #[is_not_empty]
    id: String,
}

#[singleton]
#[derive(Default)]
struct OrderTools;

#[tools]
impl OrderTools {
    /// The description is what the model reads to decide whether to
    /// call this. Write it for the model, not for a colleague.
    #[tool("Look up an order's status by its id")]
    async fn read_order(&self, args: ReadOrder) -> serde_json::Value {
        // Return the *shape* you want the model to read. A `Result`
        // would reach it as {"Ok": …}.
        serde_json::json!({ "id": args.id, "status": "shipped" })
    }
}

struct SupportMindset;

#[async_trait]
impl Mindset for SupportMindset {
    /// Everything the persona is, in one await — which is what makes
    /// it cacheable and stops the operator seeing a half-updated one.
    async fn describe(&self) -> MindsetDescription {
        MindsetDescription::new(
            MindsetIdentity::new("Support", "english").with_personality("brief and warm"),
        )
        .with_context("Orders ship within two working days.")
        .with_skills("track orders, explain delays")
        .with_limits("Never promise a refund; escalate instead.")
    }

    async fn models(&self) -> MindsetModels {
        MindsetModels::new().with(ModelKind::Llm, vec![MindsetModelRef::new("claude-opus-5")])
    }
}
```

Wire it per chat, because each chat needs its own memory:

```rust
// Note the argument order: the container comes first.
let operator = MindsetOperator::new(container.clone(), Arc::new(SupportMindset))
    .with_module_tools(OrderTools::register_tools(&container));

let bot = ChatBot::new(memory, adapter, Arc::new(operator));
bot.send_message(ChatMessage::text("where is my order?"), reply).await?;
```

## 4. Background work

```rust
#[command("send-email")]
#[derive(Serialize, Deserialize, Validate)]
struct SendEmail {
    #[is_not_empty]
    to: String,
}

#[injectable]
#[derive(Default)]
struct SendEmailHandler;

#[command_handler(SendEmail, retry_delays = [5, 30])]
impl SendEmailHandler {
    async fn handle(&self, data: SendEmail) -> Result<(), AsyncError> {
        tracing::info!(to = %data.to, "sending");
        Ok(())
    }
}

// Enqueue from anywhere that has the container:
run_command(&container, &SendEmail { to: "a@b.c".into() }).await?;
```

Cron uses **six** fields, seconds first: `"0 0 2 * * *"` is 02:00
daily. A five-field string parses and means something else.

## 5. Putting it together with one lifecycle

```rust
use wabot::ProjectRunner;
use wabot_core::lifecycle::{ShutdownPhase, ShutdownTask};

#[tokio::main]
async fn main() {
    let c = Container::new();
    register_singletons!(&c, UserService, UserController, SendEmailHandler);

    let router = UserController::register_routes(&c, wabot::rest::axum::Router::new());
    let pool = /* PgDatabase::connect(...).await?.pool().clone() */;

    let outcome = ProjectRunner::new(c.clone())
        .service("rest", run_rest_controllers(router, RestServerConfig::from_env()))
        .service("jobs", run_async_workers(c.clone(), vec![SendEmailHandler::__handler_entry(&c)], vec![]))
        .on_shutdown(ShutdownTask::new("pool", ShutdownPhase::Close, move || {
            let pool = pool.clone();
            async move { pool.close().await }
        }))
        .run()
        .await;

    std::process::exit(outcome.exit_code());
}
```

The runner gives you one signal handler, one drain order, and a
process that stops when a service does — a REST server that died
because its port was taken must not leave something looking healthy.

## 6. Test it

```rust
// tests/users.rs
use wabot::testing::RestHarness;

#[tokio::test]
async fn a_user_round_trips() {
    let c = Container::new();
    register_singletons!(&c, UserService, ApiKeyMiddleware, UserController);
    let harness = RestHarness::new(
        UserController::register_routes(&c, wabot::rest::axum::Router::new())
    );

    let created: User = harness
        .post("/api/users/")
        .header("x-api-key", "secret")
        .json(&serde_json::json!({ "name": "Ada" }))
        .send()
        .await
        .json();     // panics with the status and body if it wasn't 2xx

    assert_eq!(created.name, "Ada");
}
```

No port is bound: an axum router *is* a `tower::Service`, so the
request goes straight through it.

## Where to go next

| You are doing | Read |
| --- | --- |
| anything, first | `wabot-rust-framework` — DI lifetimes, the macro set, what is deliberately absent |
| storing data | `wabot-rust-persistence` |
| endpoints, sockets, pages | `wabot-rust-controllers` |
| mindsets, tools, agents, adapters | `wabot-rust-llm` |
| jobs, cron, shutdown | `wabot-rust-async` |
| tests | `wabot-rust-testing` |

`CLAUDE.md` at the workspace root has the per-phase design notes,
including the mistakes and why each decision went the way it did.
Read the relevant section before changing framework code.
