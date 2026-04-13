# Projections

A projection replays events for an entity and folds them into typed state. Implement `EventSourced`, then call `store.project()`.

## Complete example

```rust
use batpak::prelude::*;
use batpak::store::{Store, StoreConfig, Freshness};

#[derive(Default, Debug, serde::Serialize, serde::Deserialize)]
struct HitCounter {
    count: u64,
}

impl EventSourced for HitCounter {
    type Input = ValueInput; // decode payloads to serde_json::Value

    fn from_events(events: &[Event<serde_json::Value>]) -> Option<Self> {
        if events.is_empty() { return None; }
        let mut state = Self::default();
        for event in events {
            state.apply_event(event);
        }
        Some(state)
    }

    fn apply_event(&mut self, _event: &Event<serde_json::Value>) {
        self.count += 1;
    }

    fn relevant_event_kinds() -> &'static [EventKind] {
        static KINDS: [EventKind; 1] = [EventKind::custom(0xF, 1)];
        &KINDS
    }
}

let store = Store::open(StoreConfig::new("./data"))?;
let coord = Coordinate::new("counter:hits", "app:web")?;

// Append some events
store.append(&coord, EventKind::custom(0xF, 1), &serde_json::json!({"x": 1}))?;
store.append(&coord, EventKind::custom(0xF, 1), &serde_json::json!({"x": 2}))?;

// Project: replays events and folds into HitCounter
let state: Option<HitCounter> = store.project("counter:hits", &Freshness::Consistent)?;
assert_eq!(state.unwrap().count, 2);
```

## Which projection API to use

| Situation | API | Notes |
|---|---|---|
| One-shot state reconstruction | `store.project(entity, &freshness)` | Replays from cache or disk |
| Skip unchanged entities | `store.project_if_changed(entity, last_gen, &freshness)` | Returns `None` if generation hasn't bumped |
| Check entity generation | `store.entity_generation(entity)` | Cheap u64 check, no replay |
| Live reactive projection | `store.watch_projection(entity, freshness)` on `Arc<Store>` | Blocks on `watcher.recv()` until new events arrive |
| Live lossy fold | `subscribe_lossy().ops().scan(initial, fold_fn)` | Dashboard-grade, may skip events |
| Guaranteed fold | `store.cursor_worker(region, config, handler)` | Restartable, lossless |

## Freshness

- `Freshness::Consistent` — always replays from the latest events. Cache is used only if it matches the current watermark.
- `Freshness::MaybeStale { max_stale_ms }` — accepts cached state if it was computed within `max_stale_ms`. Useful for read-heavy paths where eventual consistency is OK.

## Replay modes: ValueInput vs RawMsgpackInput

Every `EventSourced` type declares its replay mode via `type Input`:

- **`ValueInput`** (default): payloads are decoded to `serde_json::Value`. Simple, flexible, works with any JSON-compatible payload. Use this unless you have a measured cold-replay bottleneck.

- **`RawMsgpackInput`**: payloads stay as raw `Vec<u8>` MessagePack bytes. Your `apply_event` receives `&Event<Vec<u8>>` and you deserialize only the fields you need via `rmp_serde::from_slice`. Skips the full JSON value tree allocation — use this when cold-replay throughput matters and your projection only reads a few fields per event.

```rust
// Value mode (default):
impl EventSourced for MyProjection {
    type Input = ValueInput;
    fn apply_event(&mut self, event: &Event<serde_json::Value>) { ... }
    // ...
}

// Raw mode (throughput-sensitive):
impl EventSourced for MyProjection {
    type Input = RawMsgpackInput;
    fn apply_event(&mut self, event: &Event<Vec<u8>>) {
        let payload: MyPayload = rmp_serde::from_slice(&event.payload).unwrap();
        // only decode what you need
    }
    // ...
}
```

The store automatically selects the correct replay lane based on your `Input` type. Both modes share the same cache, versioning, and incremental replay behavior.

## Cache versioning

Override `fn schema_version() -> u64` on your `EventSourced` type to isolate cached projections across schema changes. When you bump the version, old cached entries are ignored and a fresh replay runs. Different types always get separate cache slots (keyed by `TypeId`).

## Incremental replay

For types where `from_events` is a pure fold over `apply_event`, opt in to incremental replay:

1. Override `fn supports_incremental_apply() -> bool { true }` on your type
2. Set `StoreConfig::with_incremental_projection(true)`

When enabled, stale cached state is loaded and only new events (since the cached watermark) are applied — no full replay.

## What happens under the hood

1. **First call** (cold): store builds a replay plan from the index, reads matching events from disk, calls `from_events()`, caches the result. ~8-15ms for 1k events.
2. **Subsequent calls** (warm): group-local cache hit returns in ~10μs. No disk I/O.
3. **After new events**: cache is stale. If incremental is enabled, only new events are applied (~100μs). Otherwise, full replay from cache or disk.
