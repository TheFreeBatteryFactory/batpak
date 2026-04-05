# Projections

Implement `EventSourced`, then call `Store::project`. Use `Freshness::Consistent` when you need exact replay and a cache backend when replay cost matters.

## Reactive projections

Use `Store::watch_projection` (on `Arc<Store>`) to get a `ProjectionWatcher<T>` that automatically re-projects whenever new events arrive for the entity. Call `watcher.recv()` to block until the next update.

## Cache versioning

Override `fn schema_version() -> u64` on your `EventSourced` type to isolate cached projections across schema changes. When you bump the version, old cached entries are ignored and a fresh replay runs. Different types always get separate cache slots (keyed by `TypeId`).

## Incremental replay

For types where `from_events` is a pure fold over `apply_event`, opt in to incremental replay:

1. Override `fn supports_incremental_apply() -> bool { true }` on your type
2. Set `StoreConfig::with_incremental_projection(true)`

When enabled, stale cached state is loaded and only new events (since the cached watermark) are applied — no full replay.
