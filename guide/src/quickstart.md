# Quickstart

batpak is a sync-first event store for Rust. No async runtime. Events go in, projections come out.

```bash
cargo add batpak
```

## Minimal example

```rust
use batpak::prelude::*;
use batpak::store::{Store, StoreConfig, Freshness};

// 1. Open a store
let store = Store::open(StoreConfig::new("./my-data"))?;

// 2. Append events
let coord = Coordinate::new("counter:hits", "app:web")?;
let kind = EventKind::custom(0xF, 1);
store.append(&coord, kind, &serde_json::json!({"delta": 1}))?;
store.append(&coord, kind, &serde_json::json!({"delta": 3}))?;

// 3. Query events
let events = store.stream("counter:hits");
assert_eq!(events.len(), 2);

// 4. Project state (optional — see Projections guide)
// let state: Option<Counter> = store.project("counter:hits", &Freshness::Consistent)?;

// 5. Close cleanly
store.close()?;
```

## What's in the box

| Feature | API | Guide page |
|---|---|---|
| Append events | `store.append()`, `store.append_batch()` | [Append and query](append-and-query.md) |
| Query events | `store.query()`, `store.stream()`, `store.by_fact()` | [Append and query](append-and-query.md) |
| Reconstruct state | `store.project()`, `store.project_if_changed()` | [Projections](projections.md) |
| Nonblocking writes | `store.submit()`, `store.try_submit()` | [Control plane](control-plane.md) |
| Batch staging | `store.outbox()` | [Control plane](control-plane.md) |
| Atomic visibility | `store.begin_visibility_fence()` | [Control plane](control-plane.md) |
| Live subscriptions | `store.subscribe_lossy()`, `store.cursor_guaranteed()` | [Subscriptions](subscriptions.md) |
| Policy enforcement | `GateSet`, `Pipeline` | [Policy gates](policy-gates.md) |
| Read-only access | `Store::<ReadOnly>::open_read_only()` | [Control plane](control-plane.md) |
| Backpressure | `store.writer_pressure()`, `store.try_submit()` | [Control plane](control-plane.md) |

## Running the examples

```bash
cargo run --example quickstart
cargo run --example event_sourced_counter
cargo run --example chat_room
cargo run --example batch_append
cargo run --example submit_pipeline
cargo run --example outbox
cargo run --example visibility_fence
cargo run --example cursor_worker
cargo run --example read_only
cargo run --example raw_projection_counter
```
