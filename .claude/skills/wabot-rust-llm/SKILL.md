---
name: wabot-rust-llm
description: Use when building anything LLM-mediated in wabot-rust — a mindset, a tool set, an agent, or a chat adapter for a new provider. Covers the Mindset trait with describe()/models()/cache(), #[tools] and #[tool] and how a schema is derived from #[derive(Validate)], the ToolGate allow/deny precedence that decides which tools a mindset may reach, why tool failures come back as payloads the model can correct rather than errors that abort the turn, the required-but-nullable treatment of optional arguments and its per-provider spelling, agents with ask/confirm/order and typed answers, mindset-to-agent delegation and the DI seam that breaks the crate cycle, and what each of the six chat adapters does differently — including the multimodal mapping that is still missing in five of them.
---

# Mindsets, tools and agents

## Mindset

```rust
#[async_trait]
impl Mindset for SupportMindset {
    async fn describe(&self) -> MindsetDescription {
        MindsetDescription::new(
            MindsetIdentity::new("Support", "english").with_personality("brief and warm"),
        )
        .with_context("Orders ship within two working days.")
        .with_limits("Never promise a refund.")
    }

    async fn models(&self) -> MindsetModels {
        MindsetModels::new().with(ModelKind::Llm, vec![MindsetModelRef::new("claude-opus-5")])
    }

    /// Opt-in; see below for why.
    fn cache(&self) -> Option<MindsetCacheConfig> { None }
}

// The container comes first.
let operator = MindsetOperator::new(container.clone(), Arc::new(SupportMindset))
    .with_module_tools(OrderTools::register_tools(&container));
```

`describe()` is the only required method, and it returns everything in
one await — that is what makes
caching possible and stops the operator observing a half-updated
persona.

**Caching is opt-in.** A `describe()` that reads per-chat state would
otherwise serve one chat's context to every other chat. Keyed by
`cache_key()`, whose default is the type name.

## Tools

```rust
#[tools]
impl OrderTools {
    #[tool("Read an order by id")]
    async fn read_order(&self, args: ReadOrder) -> OrderView { … }
}
```

The schema comes from `<ReadOrder as Validate>::model_info()` — the
same declaration the validator uses. Nothing to keep in sync.

**A tool handler cannot signal failure with `Result`**: the macro
serializes whatever the method returns, so a `Result` would reach the
model as `{"Ok":…}`. Return the error *shape* you want it to read.

**Failures come back as payloads, not errors.** Malformed JSON
arguments, a validation failure, a blocked tool and a handler that
errored all return a JSON object naming the problem, so the model can
re-issue the call with the field corrected. Only an *unknown* tool is
an `Err` — the model invented a name and there is nothing to dispatch
to.

`ToolGate` is the safety boundary: a set marked
`expose_to_mindsets = false` is invisible to a mindset unless a
delegation allow-lists it by name. **Deny beats allow.** A divergence
here is an authorization difference, not a behaviour difference.

## Optional arguments are required-but-nullable

Every parameter is advertised as required, and the optional ones are
marked nullable. OpenAI's strict mode demands it, and a model handed a
genuinely optional parameter either omits it silently or invents a
plausible value; making it answer `null` turns "I have no value" into
an explicit choice.

`normalize_optional_arguments` undoes it at the boundary: a null, an
empty string or a blank one on a property declared optional **drops the
key**, so it deserializes as `None` rather than `Some("")`. Required
properties are never touched.

Nullability is spelled per provider — JSON-Schema unions for
Anthropic/OpenAI/DeepSeek/OpenRouter, `"nullable": true` for Gemini.
Each adapter has a test asserting its own spelling.

## Agents

```rust
// `ask` advances the transcript, so the session is taken by &mut.
let mut session = factory.for_agent(agent).session().await;
let answer: Plan = session.ask::<Plan>("draft a plan").await?;
let yes = session.confirm("is this safe?").await?;
```

The answer schema is a synthetic `__wabot_final_answer` tool, and a
**rejected answer goes back to the model to correct**, not to the
caller as an error — the same principle as tool arguments.

Prose where a schema was requested is `AgentError::Question`, not a
failure: an agent asking for clarification is normal.

**Delegation from a mindset is deliberately narrow**, because the
person on the other end of the chat is not the developer: privileged
tool sets stay hidden, each call gets a fresh session with **no chat
history**, and the budget is capped at 4000 tokens / 8 steps.

The mindset↔agent cycle is broken with a DI seam
(`register_agents(&c, adapter)`): in Rust a cyclic crate dependency
simply doesn't compile.

## Adapters

Six, each a direct `reqwest` call to one endpoint — no provider SDKs.
Anthropic (Messages), OpenAI (**Responses** API), Google (Gemini
`generateContent`), DeepSeek and OpenRouter (Chat Completions), and the
Wabot proxy.

Retry walks `req.models` in order on 408/429/5xx; 400/401/403 abort.

**Still missing in five of six: the multimodal mapping.** Only
OpenRouter maps images and documents; the rest drop them. The
conformance suite names this gap rather than testing around it.

## Verifying a new adapter

Write a mock-server test for its dialect, then run
`wabot_testing::conformance::chat_adapter_conformance` against the real
provider. The mock proves it speaks the dialect; the conformance suite
proves it behaves like the other five.
