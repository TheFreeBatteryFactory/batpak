# Projections

Implement `EventSourced`, then call `Store::project`. Use `Freshness::Consistent` when you need exact replay and a cache backend when replay cost matters.

## Reactive projections

Use `Store::watch_projection` (on `Arc<Store>`) to get a `ProjectionWatcher<T>` that automatically re-projects whenever new events arrive for the entity. Call `watcher.recv()` to block until the next update.

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
