---
name: wabot-rust-controllers
description: Use when exposing something over the network in wabot-rust — a REST endpoint, a Socket.IO controller, a server-rendered UI page, or a chat channel. Covers #[rest_controller] with its typed request merging and RestError status mapping, middleware as rejection-only plus when to use a tower layer instead, #[socket_controller] with handshake middlewares as a connect middleware and the ordering rules that make events arrive at all, the per-connection child container, #[ui_controller] with views, actions, islands, static generation and boosted navigation, #[chat_controller] with its per-channel markers, and the auth guards for each — jwt_guard!, ApiKeyGuard, jwt_handshake_guard! with its mandatory origin allowlist for cookie auth, and why a controller reading Auth must be registered transient.
---

# Controllers

Four kinds, all impl-block macros. They share `Middleware` and
`RestError`.

## REST

```rust
#[singleton]
#[derive(Default)]
struct UserController { users: Arc<UserService> }

#[rest_controller("/users")]
impl UserController {
    #[get("/:id")]
    #[middleware(JwtGuard)]
    async fn find(&self, req: FindUser) -> RestResult<UserView> { … }
}

let router = UserController::register_routes(&container, Router::new());
run_rest_controllers(router, RestServerConfig::from_env()).await?;
```

Path, query and body are **merged into one struct** that derives
`Deserialize + Validate`. There is no `RestRequest` marker: the TS one
exists because Express conflates the request with the body.

URL-borne values arrive as text. Numeric path/query params need a
`String` field parsed manually, or `#[serde(deserialize_with)]`.

**Middleware is rejection-only** (`async fn handle(&self, parts,
container) -> RestResult<()>`). It can refuse but not touch the
response; for CORS, gzip or rate limiting, add a tower layer to the
router — a strict superset of what TS middleware can do.

Start from the framework's stack, not a bare router:
`run_rest_controllers` and `RestHarness` both build it with
`rest_app()`, which adds trailing-slash normalization and the request
log context.

## Auth

```rust
jwt_guard!(AdminGuard, audience = "admin");
#[middleware(AdminGuard)]
```

**A controller that reads `Auth` must be `register_transients!`.**
`Auth` lives in the request's child container; a singleton controller
is built once, so its `Arc<Auth>` would be one caller's identity served
to everyone.

Audience is enforced by the framework, not the library: `jsonwebtoken`
skips the `aud` check when a token carries no audience at all.

API-key secrets are stored hashed and looked up **by hash**, so
database read access alone doesn't authenticate. A storage failure is a
500, never a 401 — a broken database must not look like a bad
credential.

## Sockets

```rust
#[socket_controller("/rooms", handshake(TenantHandshake))]
impl RoomController {
    #[on_socket_event("connection")]
    async fn on_connect(&self, socket: SocketRef) -> SocketResult<()> { … }

    #[on_socket_event("join")]
    async fn join(&self, req: JoinRoom, socket: SocketRef) -> SocketResult<Joined> { … }
}
```

Two ordering rules, both learned by every event test timing out:

- **The handshake runs as a connect *middleware***, not at the top of
  the connect handler. A middleware runs before the client is told the
  connection succeeded; anything awaited inside the handler leaves a
  window where the client is already emitting and no listener is
  attached, and those packets are dropped silently.
- **The connect handler is synchronous.** An async one is polled when
  the runtime gets to it, which can be after the client's first emit.

A refused handshake means **no events are ever wired**. Each connection
gets a child container built at handshake time; a bad payload is
answered as `{ error: { code, message } }` through the ack, not raised.

## UI

`#[ui_controller("/path", app, layout)]` with `#[view("/x")]` and
`#[action("save")]`. A view returns a `ViewBody` and declares islands,
styles and title through a `RenderScope` — a `tokio::task_local`, so it
survives every `.await` a handler makes.

`#[view("/x", static)]` renders once; `revalidate = N` makes it ISR.
**`static` + `#[middleware]` is a compile error**: a cached page is
rendered once and shared, so a guard on it would be silently bypassed.

## Chat channels

`#[chat_controller]` on the impl block, with a per-method channel
marker: `#[cmd]`, `#[telegram]`, `#[socket]`. Each marker maps to an
addon crate resolved by the macro. Methods inside the block look dead
to rustc (calls go through type-erased closures), so examples put
`#![allow(dead_code)]` at the top.
