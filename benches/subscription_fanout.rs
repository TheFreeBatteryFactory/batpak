//! Benchmark: subscriber fanout under append load.
//!
//! [SPEC:benches/subscription_fanout.rs]

mod common;

use batpak::prelude::*;
use batpak::store::{Store, StoreConfig};
use common::{apply_profile, throughput_elements, BenchProfile};
use criterion::{criterion_group, criterion_main, BatchSize, BenchmarkId, Criterion};
use tempfile::TempDir;

fn bench_append_with_subscribers(c: &mut Criterion) {
    let mut group = c.benchmark_group("append_with_subscribers");
    apply_profile(&mut group, BenchProfile::Heavy);

    for subscribers in [1usize, 10, 100] {
        let event_count = 10_000u64;
        throughput_elements(&mut group, event_count);
        group.bench_with_input(
            BenchmarkId::new("subscribers", subscribers),
            &subscribers,
            |b, &subscribers| {
                b.iter_batched(
                    || {
                        let dir = TempDir::new().expect("create temp dir");
                        let config = StoreConfig {
                            data_dir: dir.path().to_path_buf(),
                            broadcast_capacity: 20_000,
                            ..StoreConfig::new("")
                        };
                        let store = Store::open(config).expect("open store");
                        let region = Region::entity("fanout:entity");
                        for _ in 0..subscribers {
                            let _ = store.subscribe_lossy(&region);
                        }
                        let coord =
                            Coordinate::new("fanout:entity", "fanout:scope").expect("valid");
                        let kind = EventKind::custom(0xF, 1);
                        (store, dir, coord, kind)
                    },
                    |(store, dir, coord, kind)| {
                        for i in 0..event_count {
                            store
                                .append(&coord, kind, &serde_json::json!({"i": i}))
                                .expect("append");
                        }
                        (store, dir)
                    },
                    BatchSize::SmallInput,
                );
            },
        );
    }

    group.finish();
}

fn bench_drain_notifications(c: &mut Criterion) {
    let mut group = c.benchmark_group("drain_notifications");
    apply_profile(&mut group, BenchProfile::Heavy);

    for subscribers in [1usize, 10, 100] {
        let event_count = 10_000u64;
        throughput_elements(&mut group, event_count * subscribers as u64);
        group.bench_with_input(
            BenchmarkId::new("subscribers", subscribers),
            &subscribers,
            |b, &subscribers| {
                b.iter_batched(
                    || {
                        let dir = TempDir::new().expect("create temp dir");
                        let config = StoreConfig {
                            data_dir: dir.path().to_path_buf(),
                            broadcast_capacity: 20_000,
                            ..StoreConfig::new("")
                        };
                        let store = Store::open(config).expect("open store");
                        let coord =
                            Coordinate::new("fanout:entity", "fanout:scope").expect("valid");
                        let kind = EventKind::custom(0xF, 1);
                        let region = Region::entity("fanout:entity");
                        let mut receivers = Vec::new();
                        for _ in 0..subscribers {
                            receivers.push(store.subscribe_lossy(&region));
                        }
                        for i in 0..event_count {
                            store
                                .append(&coord, kind, &serde_json::json!({"i": i}))
                                .expect("append");
                        }
                        store.sync().expect("sync");
                        (store, dir, receivers)
                    },
                    |(store, dir, receivers)| {
                        for rx in receivers {
                            let mut seen = 0u64;
                            while seen < event_count {
                                if rx.recv().is_some() {
                                    seen += 1;
                                } else {
                                    break;
                                }
                            }
                        }
                        (store, dir)
                    },
                    BatchSize::SmallInput,
                );
            },
        );
    }

    group.finish();
}

criterion_group!(
    benches,
    bench_append_with_subscribers,
    bench_drain_notifications
);
criterion_main!(benches);
