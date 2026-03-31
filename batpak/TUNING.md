# Tuning Guide

All settings live in `StoreConfig`. Create one via `StoreConfig::new(data_dir)` which
applies sane defaults, then override fields before passing to `Store::open()`.

## Configuration Reference

| Field | Default | Unit | When to change |
|-------|---------|------|----------------|
| `segment_max_bytes` | 256 MB | bytes | Lower for embedded (e.g. 16 MB). Higher if events are large and you want fewer files. |
| `sync_every_n_events` | 1000 | events | Lower (1-10) for strict durability. Higher (5000+) for throughput-first workloads. |
| `fd_budget` | 64 | count | Raise if you have many segments open concurrently. Lower on FD-constrained systems. |
| `writer_channel_capacity` | 4096 | messages | Back-pressure threshold. Raise if producers are bursty. Lower to surface pressure earlier. |
| `broadcast_capacity` | 8192 | messages | Per-subscriber lossy ring. Raise for slow consumers. Lower to bound memory per subscriber. |
| `cache_map_size_bytes` | 64 MB | bytes | LMDB map size for projection cache. Raise if projections are large. |
| `restart_policy` | `RestartPolicy::default()` | - | Controls writer thread recovery from panics. |
| `shutdown_drain_limit` | 1024 | messages | Max queued commands drained on shutdown. Raise if shutdown drops events. |
| `writer_stack_size` | `None` (OS default) | bytes | Set explicitly if writer thread needs deep stacks (recursive projections). |
| `clock` | `None` (SystemTime) | - | Inject deterministic clock for testing. Returns microseconds since epoch. |
| `sync_mode` | `SyncAll` | - | `SyncData` skips metadata fsync (faster, slightly less crash-safe). |

## Tradeoff Matrix

### Durability vs Throughput
- `sync_every_n_events = 1` + `sync_mode = SyncAll` = maximum durability, lowest throughput
- `sync_every_n_events = 5000` + `sync_mode = SyncData` = maximum throughput, window of loss on crash

### Memory vs File Handles
- High `fd_budget` = more segments memory-mapped = faster reads, more FDs
- Low `fd_budget` = more LRU eviction = fewer FDs, occasional re-open cost

### Back-pressure vs Latency
- High `writer_channel_capacity` = producers rarely block, higher peak memory
- Low `writer_channel_capacity` = producers block early, bounded memory, higher tail latency

## Example: Embedded Device

```rust
let mut config = StoreConfig::new("/data/events");
config.segment_max_bytes = 16 * 1024 * 1024;  // 16 MB segments
config.fd_budget = 8;                           // minimal FDs
config.sync_every_n_events = 10;                // near-durable
config.writer_channel_capacity = 256;           // tight memory
config.cache_map_size_bytes = 4 * 1024 * 1024;  // 4 MB cache
```

## Example: High-Throughput Server

```rust
let mut config = StoreConfig::new("/var/lib/events");
config.segment_max_bytes = 1024 * 1024 * 1024;  // 1 GB segments
config.sync_every_n_events = 5000;               // batch syncs
config.sync_mode = SyncMode::SyncData;           // skip metadata
config.fd_budget = 256;                          // many open segments
config.writer_channel_capacity = 16384;          // high burst tolerance
config.broadcast_capacity = 32768;               // slow consumer tolerance
```

## Projection Cache Backends

`Store::open` uses `NoCache` by default. For `project()` workloads that repeatedly fold
large event streams, use a persistent cache backend via `Store::open_with_cache`.

### LmdbCache (`feature = "lmdb"`)

```rust
let mut config = StoreConfig::new("/var/lib/events");
config.cache_map_size_bytes = 128 * 1024 * 1024;  // 128 MB LMDB map
let store = Store::open_with_cache(
    config.clone(),
    Box::new(LmdbCache::open("/var/lib/events-cache", config.cache_map_size_bytes)?),
)?;
```

## Benchmark Surfaces

Use separate surfaces for backend-neutral and backend-specific performance work:

- `just bench-neutral` measures reopen/replay/write/fanout/compaction without cache backend noise
- `just bench-redb` measures redb-backed projection cache behavior
- `just bench-lmdb` measures LMDB-backed projection cache behavior

Save or compare baselines per OS and surface:

```bash
just bench-save surface=neutral
just bench-compare surface=neutral
just bench-save surface=lmdb
```

LMDB cache-miss benchmarks are now Windows-safe because they reuse one environment and
measure uncached entities instead of creating a fresh LMDB environment per sample.
