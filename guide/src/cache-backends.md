# Cache backends

Projection caching makes warm-path `project()` calls return in ~10μs instead of replaying from disk.

## Built-in backends

### NoCache (default)

```rust
let store = Store::open(config)?; // uses NoCache
```

Every `project()` call replays from disk. Simple, correct, no external state. Use this when projections are called infrequently or replay cost doesn't matter.

### NativeCache (file-backed)

```rust
let store = Store::open_with_native_cache(config, "./cache")?;
```

Caches serialized projection state to disk, sharded by entity. On `project()`:
1. Check group-local in-memory cache (fastest, ~10μs)
2. Check NativeCache on disk (~100μs)
3. Replay from segments (slowest, ~8ms for 1k events)

Cache files are intentionally NOT durable to disk loss — they're rebuildable from segments. Corrupt cache files are deleted automatically (self-healing).

## Freshness control

```rust
use batpak::store::Freshness;

// Always replay if cache is stale (exact state):
store.project::<Counter>("entity:1", &Freshness::Consistent)?;

// Accept cache if < 5 seconds old (eventual consistency):
store.project::<Counter>("entity:1", &Freshness::MaybeStale { max_stale_ms: 5_000 })?;
```

## Custom cache backends

Implement `ProjectionCache`:

```rust
use batpak::store::{ProjectionCache, CacheCapabilities, CacheMeta, StoreError};

struct MyCache { /* ... */ }

impl ProjectionCache for MyCache {
    fn capabilities(&self) -> CacheCapabilities {
        CacheCapabilities {
            supports_prefetch: false,
            is_noop: false,
        }
    }

    fn get(&self, key: &[u8]) -> Result<Option<(Vec<u8>, CacheMeta)>, StoreError> {
        // look up by key, return (serialized_state, meta) or None
        Ok(None)
    }

    fn put(&self, key: &[u8], value: &[u8], meta: CacheMeta) -> Result<(), StoreError> {
        // store serialized state + meta
        Ok(())
    }

    fn delete_prefix(&self, prefix: &[u8]) -> Result<u64, StoreError> {
        Ok(0) // delete cache entries matching prefix
    }

    fn sync(&self) -> Result<(), StoreError> {
        Ok(()) // flush to durable storage
    }
}
```

Then pass it to `Store::open_with_cache(config, Box::new(MyCache { ... }))`.

## Cache key composition

Cache keys are constructed as: `entity_bytes + \0 + TypeId_hash(u64 LE) + schema_version(u64 LE)`. This guarantees:
- Different entity names never collide
- Different projection types on the same entity get separate slots
- Schema version bumps invalidate old entries
