//! Projection replay and cache benchmarks with explicit cold/hot lanes.

mod common;

use batpak::prelude::*;
use batpak::store::{Freshness, Store, StoreConfig, ViewConfig};
use common::{apply_profile, throughput_elements, BenchProfile};
use criterion::{criterion_group, criterion_main, BatchSize, Criterion};
use tempfile::TempDir;

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

fn populate_projection_fixture(store: &Store, entity: &str, events: u64) {
    let coord = Coordinate::new(entity, "bench:scope").expect("valid coord");
    let kind = EventKind::custom(0xF, 1);
    let payload = serde_json::json!({"x": 1});
    for _ in 0..events {
        store.append(&coord, kind, &payload).expect("append");
    }
}

fn bench_projection_lanes(c: &mut Criterion) {
    let mut group = c.benchmark_group("projection_lanes");
    apply_profile(&mut group, BenchProfile::Quick);
    throughput_elements(&mut group, 1_000);

    let fixture_dir = TempDir::new().expect("create temp dir");
    let config = StoreConfig {
        data_dir: fixture_dir.path().to_path_buf(),
        ..StoreConfig::new("")
    };
    let store = Store::open(config).expect("open store");
    populate_projection_fixture(&store, "bench:first-pass", 1_000);
    store.sync().expect("sync");
    store.close().expect("close");

    // project-only: measures ONLY the projection call, not close().
    // close() is in the drop path outside the measured routine.
    group.bench_function("projection_first_pass", |b| {
        b.iter_batched(
            || {
                let config = StoreConfig {
                    data_dir: fixture_dir.path().to_path_buf(),
                    ..StoreConfig::new("")
                };
                Store::open(config).expect("reopen populated store")
            },
            |store| {
                let _: Option<Counter> = store
                    .project("bench:first-pass", &Freshness::Consistent)
                    .expect("project");
                // close() deliberately excluded from timing — it writes cold-start
                // artifacts which is lifecycle cost, not projection cost. The Store
                // drops at the end of this closure, which triggers best-effort
                // shutdown without artifact writes (should_shutdown_on_drop path).
            },
            BatchSize::SmallInput,
        );
    });

    // project + close: measures the full lifecycle including artifact writes.
    // Kept as a separate lane so "projection cost" and "lifecycle cost" are
    // never conflated.
    group.bench_function("projection_first_pass_with_close", |b| {
        b.iter_batched(
            || {
                let config = StoreConfig {
                    data_dir: fixture_dir.path().to_path_buf(),
                    ..StoreConfig::new("")
                };
                Store::open(config).expect("reopen populated store")
            },
            |store| {
                let _: Option<Counter> = store
                    .project("bench:first-pass", &Freshness::Consistent)
                    .expect("project");
                store.close().expect("close");
            },
            BatchSize::SmallInput,
        );
    });

    let replay_dir = TempDir::new().expect("create replay temp dir");
    let replay_config = StoreConfig::new(replay_dir.path())
        .with_views(ViewConfig::none())
        .with_enable_checkpoint(false)
        .with_enable_mmap_index(false);
    let replay_store = Store::open(replay_config).expect("open replay store");
    let replay_entities: Vec<String> = (0..256).map(|i| format!("bench:replay-only:{i}")).collect();
    for entity in &replay_entities {
        populate_projection_fixture(&replay_store, entity, 64);
    }
    replay_store.sync().expect("sync replay-only store");

    let mut replay_index = 0usize;
    group.bench_function("projection_replay_only", |b| {
        b.iter(|| {
            let entity = &replay_entities[replay_index % replay_entities.len()];
            replay_index += 1;
            let _: Option<Counter> = replay_store
                .project(entity, &Freshness::Consistent)
                .expect("project replay only");
        });
    });

    let dir = TempDir::new().expect("create temp dir");
    let config = StoreConfig {
        data_dir: dir.path().join("data"),
        ..StoreConfig::new("")
    };
    let store = Store::open_with_native_cache(config, dir.path().join("cache"))
        .expect("open store with native cache");
    populate_projection_fixture(&store, "bench:entity", 1_000);
    let miss_entities: Vec<String> = (0..64).map(|i| format!("bench:miss:{i}")).collect();
    for entity in &miss_entities {
        populate_projection_fixture(&store, entity, 1_000);
    }
    store.sync().expect("sync");
    let _: Option<Counter> = store
        .project("bench:entity", &Freshness::Consistent)
        .expect("warm native cache");

    group.bench_function("projection_cache_hit", |b| {
        b.iter(|| {
            let _: Option<Counter> = store
                .project("bench:entity", &Freshness::Consistent)
                .expect("project");
        });
    });

    let mut miss_index = 0usize;
    group.bench_function("projection_cache_miss", |b| {
        b.iter(|| {
            let entity = &miss_entities[miss_index % miss_entities.len()];
            miss_index += 1;
            let _: Option<Counter> = store
                .project(entity, &Freshness::Consistent)
                .expect("project miss");
        });
    });

    group.finish();
    replay_store.close().expect("close replay-only store");
    store.close().expect("close");
}

criterion_group!(benches, bench_projection_lanes);
criterion_main!(benches);
