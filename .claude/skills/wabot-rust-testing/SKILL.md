---
name: wabot-rust-testing
description: Use when writing tests in wabot-rust — for a mindset or agent, a REST or UI controller, a background job, a Socket.IO controller, or a chat adapter. Covers the wabot-testing crate and its one design rule that a harness wires production types together and never reimplements them, MockChatAdapter for scripting a model's decisions while the tools run for real, ChatBotHarness / AgentHarness / RestHarness / AsyncHarness / UiHarness, the two socket harnesses and when each applies, LlmJudge for properties no exact assertion can express, the chat-adapter conformance suite, and the environment variables that gate the tests needing a real database or a paid API.
---

# Testing

`wabot-testing` is off by default (`testing` feature) so a mock LLM
adapter can never reach production by accident. Add it under
`[dev-dependencies]`.

## The one rule

**A harness wires production types together; it never reimplements
them.** `ChatBotHarness` holds a real `ChatBot` and a real
`MindsetOperator`; `AgentHarness::for_agent()` returns the very
`AgentBuilder` an application uses. Anything reimplemented is a second
implementation free to drift, and a test passing against the drifted
copy is worse than no test. Two substitutes only: the model and
storage.

## Chat bots and agents

```rust
let harness = ChatBotHarness::builder(Arc::new(SupportMindset))
    .tools(OrderTools::register_tools(&container))
    .container(container)
    .build();

harness.adapter().call_tool("read_order", json!({ "id": 7 }));
harness.adapter().reply("It shipped yesterday.");

let turn = harness.send("where is my order?").await?;
assert_eq!(turn.text(), "It shipped yesterday.");
assert!(turn.called("read_order"));
```

**A tool call is scripted; the tool runs for real.** `call_tool`
scripts the model's *decision* — dispatch, argument validation and the
handler are production code.

Running out of script fails loudly, naming the usual cause: a missing
follow-up turn after a tool call, because the loop asks the model again
once the tool returns.

`ChatTurn::text()` panics unless there was exactly one reply — a turn
that replied twice is a different outcome, and taking the first would
hide it.

## REST, UI, jobs

`RestHarness` binds **no port**: an axum router *is* a `tower::Service`,
so a request goes straight through it. It builds the same stack
`run_rest_controllers` does, or an in-process harness would quietly
test an application the deployment doesn't have.

`AsyncHarness` runs the real `JobRunner`, where TS calls the handler
directly — calling the handler skips the started/succeeded transitions,
the retry decision, and the restored audit actor, which are the parts
worth pinning.

`UiHarness` asserts the server's half of an island: the host element,
its id, its props. There is no browser, so nothing hydrates.

## Sockets: two harnesses

| | `SocketHarness` | `LiveSocketHarness` |
| --- | --- | --- |
| how | drives the dispatch closure, ack captured | real port, real client |
| covers | validation, resolution, acks | handshake middlewares, `SocketRef` handlers, server emits |
| cost | no network | a bound port, `--features live-harness` |

Use the first for most tests. Use the second for the two things that
carry security weight: whether a handshake **refuses**, and anything
needing a live socket.

The live client is hand-written because `rust_socketio` 0.6 does not
interoperate with `socketioxide` 0.15. It is checked against the
official `socket.io-client@4`, which `js_client_compat.rs` runs as a
`cargo test` — if
the two disagree, the JS scripts are right.

## LlmJudge

For properties an exact assertion can't express: stayed on topic,
refused without being rude, didn't promise a refund.

```rust
judge.assert(harness.history(), "the bot gave the tracking number and stayed polite").await?;
```

The verdict is a **forced tool call**, not prose: "PASS (with
reservations)" would otherwise become whatever the parser guesses. A
judge that answers with prose anyway is an **error**, not a failure —
reporting "criteria not met" when no verdict was rendered would be a
lie about your code.

Keep judged tests few and out of the save-loop: paid, slow, and not
deterministic.

## Conformance

`chat_adapter_conformance(adapter, model)` asks every adapter the same
questions. Each was tested against its own mock server, which proves it
speaks its provider's dialect and nothing about whether they behave the
same.

## Environment

| Variable | Gates |
| --- | --- |
| `WABOT_TEST_PG_URL` | every Postgres suite; they skip without it |
| `WABOT_TEST_LLM_KEY` | `LlmJudge` and conformance |
| `WABOT_TEST_LLM_PROVIDER` | `openrouter` (default) or `openai` |

```sh
cargo test --workspace
cargo test -p wabot-feature-socket-controller --features live-harness
```
