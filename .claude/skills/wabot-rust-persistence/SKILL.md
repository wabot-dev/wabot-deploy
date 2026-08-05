---
name: wabot-rust-persistence
description: Use when storing data in wabot-rust — defining entities, choosing between the JSONB document store and the relational columnar store, writing queries, paging, indexing, running migrations, opening transactions, or building a projection. Covers Entity<D> / EntityData / EntityFields, ReadRepository vs CrudRepository, PgJsonbStore (creates its own table, promoted columns, index declarations) vs PgColumnsStore (schema governed by migrations, no DDL at all), the Query AST shared with the in-memory repository, keyset pagination and its opaque cursor, build_query_sql and why an unknown field is refused, with_transaction and its ambient connection plus savepoint nesting, the wabot-migrate CLI with checksum drift detection, ProjectionRuntime with its in-memory extension, and the traps found the hard way — camelCase JSON for wabot-ts compatibility, no serde renames on a columnar entity, and binding a query value as the column's declared type.
---

# Persistence

Two Postgres strategies behind one pair of traits, plus an in-memory
one for tests.

## Entity

```rust
#[derive(Debug, Clone, Serialize, Deserialize, Validate)]
#[serde(rename_all = "camelCase")]      // document store only — see below
struct OrderData {
    #[serde(flatten)]
    base: EntityData,
    owner_id: String,
    total: i64,
}

impl EntityFields for OrderData {
    fn entity(&self) -> &EntityData { &self.base }
    fn entity_mut(&mut self) -> &mut EntityData { &mut self.base }
}
```

`EntityData` serializes **camelCase** (`createdAt`), so a table is
readable by a wabot-ts process. That compatibility is asserted by
reading the stored JSON directly, not by round-tripping through our own
serializer — which would pass either way.

## Which store

| | `PgJsonbStore` | `PgColumnsStore` |
| --- | --- | --- |
| shape | `id`, `created_at`, `data JSONB` + promoted columns | one real column per field |
| table | **created on first use** | **created by a migration** |
| indexes | declared with `IndexDecl` | written in a migration |
| use when | the shape moves; adding a field shouldn't be a migration | something other than this app reads it: reports, joins, BI |

**A columnar entity must not use `#[serde(rename)]` or
`rename_all`.** Column names come from `ModelInfo`, which carries Rust
field names, while reads deserialize from JSON built out of those
column names. A mismatch fails on the first read with a message that
says so.

**`PgColumnsStore` issues no DDL at all** — not the table, not indexes,
not constraints. A columnar schema changes in ways no struct can imply
(an `ALTER`, a backfill, an index that costs something), and a store
that created the table would be claiming to keep it in step with the
code when it cannot.

## Queries

```rust
let open = store.query(
    &Query::new().eq("status", "open").gt("total", 100).order_desc("created_at")
).await?;
```

The `Query` AST lives in `wabot-core` beside an in-memory `matches`, so
`InMemoryRepository` and both Postgres strategies answer the same query
the same way. `build_query_sql` binds every value as `$n`; field
**names** can't be parameters, so they are checked against the known
set — **an unknown field is an error**, which closes the injection path
and turns a typo into a clear message instead of a filter that matches
everything.

A query value binds as the column's **declared** type, not its JSON
shape: an RFC-3339 string against a `TIMESTAMPTZ` is `text` to
Postgres, and the comparison is refused outright.

## Paging is keyset

`find_page(PageOptions { limit, cursor })`. The cursor carries
`(created_at, id)` — `created_at` alone is ambiguous for two rows
written in the same millisecond, and one gets dropped at a page
boundary. It is hex-encoded so callers can't depend on the encoding.
Offset paging silently repeats or skips items when the list shifts.

## Transactions

```rust
with_transaction(&pool, || async {
    orders.create(&order).await?;
    stock.update(&reserved).await?;   // both, or neither
    Ok(())
}).await?;
```

The connection is **ambient** (a task-local), so a repository written
without a thought for transactions joins one correctly. Nesting becomes
a `SAVEPOINT`; the outermost caller owns the `COMMIT`.

**Work spawned with `tokio::spawn` inside a transaction runs outside
it** — a spawned task gets no task-local. That is the right failure:
one connection cannot run two statements at once.

A transaction holds a pool slot for its whole life. Keep them short.

## Migrations

```sh
wabot-migrate create add_orders   # no database needed
wabot-migrate status              # non-zero exit on drift
wabot-migrate up
```

Plain SQL, forward-only, one transaction per migration so a retry
resumes where it stopped. An advisory lock serializes instances
starting together; a checksum stops the run if an applied migration was
edited. Never run automatically at boot.

## Projections

A projection is a *question* — a join, an aggregate, a report — not a
table, and not tied to SQL:

```rust
let revenue: Arc<dyn CustomerRevenue> = projection_for(&container)
    .from_runtime(|runtime| Arc::new(PgCustomerRevenue::new(runtime)))
    .or_in_memory(|| Arc::new(InMemoryCustomerRevenue::new(orders.clone())));
```

The in-memory half is **code you write**: there is no query to
reinterpret, only a body. Test both halves over the same data — an
extension that isn't checked against the statements it stands in for
drifts into answering a different question.

The declared shape reaches the backend because `COUNT(*)` is `bigint`,
`SUM` and `AVG` are `numeric`, and a numeric does not decode into an
`f64` on its own.

## Testing

Pg tests need `WABOT_TEST_PG_URL` and skip without it. Each creates
uniquely-named tables and drops them, so runs are parallel-safe.
