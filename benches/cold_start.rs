//! Benchmark: reopening a populated store and rebuilding the in-memory index.
//!
//! [SPEC:benches/cold_start.rs]
//!
//! This benchmark measures `Store::open()` against a copied on-disk fixture,
//! which is more honest than repeatedly reopening the same temp directory and
//! calling it "cold start".

mod common;

use batpak::prelude::*;
use batpak::store::{Store, StoreConfig};
use common::{apply_profile, profile_for_event_count, throughput_elements};
use criterion::{criterion_group, criterion_main, BatchSize, BenchmarkId, Criterion};
use tempfile::TempDir;

fn populate_store(dir: &std::path::Path, count: u64) {
    let config = StoreConfig {
        data_dir: dir.to_path_buf(),
        ..StoreConfig::new("")
    };
    let store = Store::open(config).expect("open store for populate");
    let coord = Coordinate::new("bench:entity", "bench:scope").expect("valid coord");
    let kind = EventKind::custom(0xF, 1);
    let payload = serde_json::json!({"x": 1});
    for _ in 0..count {
        store.append(&coord, kind, &payload).expect("append");
    }
    store.sync().expect("sync");
    store.close().expect("close");
}

fn copy_dir_recursive(src: &std::path::Path, dst: &std::path::Path) {
    std::fs::create_dir_all(dst).expect("create destination directory");
    for entry in std::fs::read_dir(src).expect("read source directory") {
        let entry = entry.expect("directory entry");
        let source_path = entry.path();
        let destination_path = dst.join(entry.file_name());
        if entry.file_type().expect("file type").is_dir() {
            copy_dir_recursive(&source_path, &destination_path);
        } else {
            std::fs::copy(&source_path, &destination_path).expect("copy fixture file");
        }
    }
}

fn bench_reopen_index_rebuild(c: &mut Criterion) {
    let mut group = c.benchmark_group("reopen_index_rebuild");

    for count in [1_000u64, 10_000, 100_000, 1_000_000] {
        apply_profile(&mut group, profile_for_event_count(count));
        throughput_elements(&mut group, count);

        let fixture_dir = TempDir::new().expect("create fixture temp dir");
        populate_store(fixture_dir.path(), count);

        group.bench_with_input(BenchmarkId::from_parameter(count), &count, |b, &_count| {
            b.iter_batched(
                || {
                    let iter_dir = TempDir::new().expect("create iteration dir");
                    copy_dir_recursive(fixture_dir.path(), iter_dir.path());
                    iter_dir
                },
                |iter_dir| {
                    let config = StoreConfig {
                        data_dir: iter_dir.path().to_path_buf(),
                        ..StoreConfig::new("")
                    };
                    let store = Store::open(config).expect("reopen populated store");
                    store.close().expect("close");
                },
                BatchSize::SmallInput,
            );
        });
    }

    group.finish();
}

criterion_group!(benches, bench_reopen_index_rebuild);
criterion_main!(benches);
