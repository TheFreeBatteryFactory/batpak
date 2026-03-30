//! Projection replay latency benchmarks.
//!
//! Groups:
//!   projection_replay   — NoCache (default): cold (OS page cache cold) vs warm
//!   projection_cache_redb  — RedbCache: cache hit vs cache miss [feature = "redb"]
//!   projection_cache_lmdb  — LmdbCache: cache hit vs cache miss [feature = "lmdb"]
//!
//! Run all:  cargo bench --all-features
//! Run one:  cargo bench --features redb -- projection_cache_redb

use batpak::prelude::*;
use batpak::store::{Freshness, Store, StoreConfig};
use criterion::{criterion_group, criterion_main, Criterion};
use tempfile::TempDir;

/// A minimal EventSourced implementation for benchmarking.
#[derive(Default, Debug, serde::Serialize, serde::Deserialize)]
struct Counter {
    count: u64,
}

impl EventSourced<serde_json::Value> for Counter {
    fn apply_event(&mut self, _event: &Event<serde_json::Value>) {
        self.count += 1;
    }

    fn from_events(events: &[Event<serde_json::Value>]) -> Option<Self> {
        if events.is_empty() {
            return None;
        }
        let mut state = Self::default();
        for event in events {
            state.apply_event(event);
        }
        Some(state)
    }

    fn relevant_event_kinds() -> &'static [EventKind] {
        static KINDS: [EventKind; 1] = [EventKind::custom(0xF, 1)];
        &KINDS
    }
}

fn bench_projection_replay(c: &mut Criterion) {
    let mut group = c.benchmark_group("projection_replay");

    // Setup: populate a store with events for one entity
    let dir = TempDir::new().expect("create temp dir");
    let config = StoreConfig {
        data_dir: dir.path().to_path_buf(),
        ..StoreConfig::new("")
    };
    let store = Store::open(config).expect("open store");
    let coord = Coordinate::new("bench:entity", "bench:scope").expect("valid coord");
    let kind = EventKind::custom(0xF, 1);
    let payload = serde_json::json!({"x": 1});
    for _ in 0..1_000 {
        store.append(&coord, kind, &payload).expect("append");
    }
    store.sync().expect("sync");

    // replay_cold: first projection after store open.
    // OS page cache may or may not have the segment data.
    group.bench_function("replay_cold", |b| {
        b.iter(|| {
            let _: Option<Counter> = store
                .project("bench:entity", &Freshness::Consistent)
                .expect("project");
        });
    });

    // replay_warm: project once to populate OS page cache, then measure.
    // Both runs do full segment replay (NoCache), but warm benefits from
    // OS-level page caching of the segment files.
    let _: Option<Counter> = store
        .project("bench:entity", &Freshness::Consistent)
        .expect("warm OS page cache");

    group.bench_function("replay_warm", |b| {
        b.iter(|| {
            let _: Option<Counter> = store
                .project("bench:entity", &Freshness::Consistent)
                .expect("project");
        });
    });

    group.finish();
    store.close().expect("close");
}

/// Projection cache benchmarks. Each backend gets a cache_hit and cache_miss group.
/// cache_hit: cache is pre-warmed, project reads from cache without replaying segments.
/// cache_miss: fresh cache per iteration via iter_batched — measures replay + cache write.
fn bench_projection_caches(c: &mut Criterion) {
    #[cfg(feature = "redb")]
    {
        use batpak::store::projection::RedbCache;

        let mut group = c.benchmark_group("projection_cache_redb");

        // Shared setup: 1000 events, cache pre-warmed
        let dir = TempDir::new().expect("create temp dir");
        let config = StoreConfig {
            data_dir: dir.path().join("data"),
            ..StoreConfig::new("")
        };
        let cache =
            RedbCache::open(&dir.path().join("cache.redb")).expect("open redb cache");
        let store = Store::open_with_cache(config, Box::new(cache))
            .expect("open store with redb cache");
        let coord = Coordinate::new("bench:entity", "bench:scope").expect("valid coord");
        let kind = EventKind::custom(0xF, 1);
        let payload = serde_json::json!({"x": 1});
        for _ in 0..1_000 {
            store.append(&coord, kind, &payload).expect("append");
        }
        store.sync().expect("sync");
        // Warm the cache: first project populates it
        let _: Option<Counter> = store
            .project("bench:entity", &Freshness::Consistent)
            .expect("warm redb cache");

        // cache_hit: cache is valid, no segment replay needed
        group.bench_function("cache_hit", |b| {
            b.iter(|| {
                let _: Option<Counter> = store
                    .project("bench:entity", &Freshness::Consistent)
                    .expect("project");
            });
        });

        // cache_miss: fresh cache each iteration — measures full replay + cache write
        group.bench_function("cache_miss", |b| {
            b.iter_batched(
                || {
                    let iter_dir = TempDir::new().expect("iter temp dir");
                    let cfg = StoreConfig {
                        data_dir: iter_dir.path().join("data"),
                        ..StoreConfig::new("")
                    };
                    let c = RedbCache::open(&iter_dir.path().join("c.redb"))
                        .expect("open cache");
                    let s = Store::open_with_cache(cfg, Box::new(c))
                        .expect("open store");
                    let coord =
                        Coordinate::new("bench:entity", "bench:scope").expect("coord");
                    let kind = EventKind::custom(0xF, 1);
                    let payload = serde_json::json!({"x": 1});
                    for _ in 0..1_000 {
                        s.append(&coord, kind, &payload).expect("append");
                    }
                    s.sync().expect("sync");
                    (s, iter_dir)
                },
                |(s, _dir)| {
                    let _: Option<Counter> = s
                        .project("bench:entity", &Freshness::Consistent)
                        .expect("project");
                },
                criterion::BatchSize::SmallInput,
            );
        });

        group.finish();
        store.close().expect("close");
    }

    #[cfg(feature = "lmdb")]
    {
        use batpak::store::projection::LmdbCache;
        const LMDB_MAP_SIZE: usize = 10 * 1024 * 1024; // 10 MiB

        let mut group = c.benchmark_group("projection_cache_lmdb");
        // LMDB has a per-process TLS key limit (~126 environments). SmallInput batches
        // ~11 setups per sample, so 100 samples = ~1100 environments. Use LargeInput
        // (1 setup/sample) + sample_size(10) + short warmup to stay well under the limit.
        group.sample_size(10);
        group.warm_up_time(std::time::Duration::from_millis(500));

        // Shared setup: 1000 events, cache pre-warmed
        let dir = TempDir::new().expect("create temp dir");
        let config = StoreConfig {
            data_dir: dir.path().join("data"),
            ..StoreConfig::new("")
        };
        let cache =
            LmdbCache::open(&dir.path().join("lmdb_cache"), LMDB_MAP_SIZE)
                .expect("open lmdb cache");
        let store = Store::open_with_cache(config, Box::new(cache))
            .expect("open store with lmdb cache");
        let coord = Coordinate::new("bench:entity", "bench:scope").expect("valid coord");
        let kind = EventKind::custom(0xF, 1);
        let payload = serde_json::json!({"x": 1});
        for _ in 0..1_000 {
            store.append(&coord, kind, &payload).expect("append");
        }
        store.sync().expect("sync");
        // Warm the cache: first project populates it
        let _: Option<Counter> = store
            .project("bench:entity", &Freshness::Consistent)
            .expect("warm lmdb cache");

        // cache_hit: cache is valid, no segment replay needed
        group.bench_function("cache_hit", |b| {
            b.iter(|| {
                let _: Option<Counter> = store
                    .project("bench:entity", &Freshness::Consistent)
                    .expect("project");
            });
        });

        // cache_miss: fresh cache each iteration — measures full replay + cache write
        group.bench_function("cache_miss", |b| {
            b.iter_batched(
                || {
                    let iter_dir = TempDir::new().expect("iter temp dir");
                    let cfg = StoreConfig {
                        data_dir: iter_dir.path().join("data"),
                        ..StoreConfig::new("")
                    };
                    let c = LmdbCache::open(
                        &iter_dir.path().join("lmdb_c"),
                        LMDB_MAP_SIZE,
                    )
                    .expect("open cache");
                    let s = Store::open_with_cache(cfg, Box::new(c))
                        .expect("open store");
                    let coord =
                        Coordinate::new("bench:entity", "bench:scope").expect("coord");
                    let kind = EventKind::custom(0xF, 1);
                    let payload = serde_json::json!({"x": 1});
                    for _ in 0..1_000 {
                        s.append(&coord, kind, &payload).expect("append");
                    }
                    s.sync().expect("sync");
                    (s, iter_dir)
                },
                |(s, _dir)| {
                    let _: Option<Counter> = s
                        .project("bench:entity", &Freshness::Consistent)
                        .expect("project");
                },
                criterion::BatchSize::LargeInput,
            );
        });

        group.finish();
        store.close().expect("close");
    }

    // If neither feature is enabled, this function is a no-op
    #[cfg(not(any(feature = "redb", feature = "lmdb")))]
    let _ = c;
}

criterion_group!(benches, bench_projection_replay, bench_projection_caches);
criterion_main!(benches);
