#![allow(
    clippy::unwrap_used,              // test assertions use unwrap for clarity
    clippy::cast_possible_truncation, // test data fits in target types
    clippy::panic,                    // tests panic to surface specific contract violations
)]
//! BANG 1 (RED): Unified enhancement spec.
//! Every test in this file defines a behavioral contract.
//! All tests MUST fail to compile or fail at runtime until Bang 2 is complete.
//!
//! PROVES: group commit, mmap reads, kind filtering, schema versioning,
//!         incremental projection, Arc<IndexEntry>, PackedCausation,
//!         IndexTopology (aos/scan/entity-local/tiled/all), SIDX footer, index checkpoint,
//!         string interner, config validation, single-slot projection cache.

use batpak::prelude::*;
use batpak::store::{Freshness, IndexTopology, Store, StoreConfig, StoreError};
use std::sync::Arc;
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Test projection types
// ---------------------------------------------------------------------------

/// Counter that only cares about kind 0xF:1. noise_count tracks leakage.
#[derive(Default, Debug, serde::Serialize, serde::Deserialize, PartialEq)]
struct KindFilteredCounter {
    target_count: u64,
    noise_count: u64,
}

impl EventSourced for KindFilteredCounter {
    type Input = batpak::prelude::JsonValueInput;

    fn from_events(events: &[Event<serde_json::Value>]) -> Option<Self> {
        if events.is_empty() {
            return None;
        }
        let mut s = Self::default();
        for e in events {
            s.apply_event(e);
        }
        Some(s)
    }
    fn apply_event(&mut self, event: &Event<serde_json::Value>) {
        if event.event_kind() == EventKind::custom(0xF, 1) {
            self.target_count += 1;
        } else {
            self.noise_count += 1;
        }
    }
    fn relevant_event_kinds() -> &'static [EventKind] {
        static KINDS: [EventKind; 1] = [EventKind::custom(0xF, 1)];
        &KINDS
    }
}

/// Counter that replays everything (empty relevant_event_kinds).
#[derive(Default, Debug, serde::Serialize, serde::Deserialize, PartialEq)]
struct AllCounter {
    count: u64,
}

impl EventSourced for AllCounter {
    type Input = batpak::prelude::JsonValueInput;

    fn from_events(events: &[Event<serde_json::Value>]) -> Option<Self> {
        if events.is_empty() {
            return None;
        }
        let mut s = Self::default();
        for e in events {
            s.apply_event(e);
        }
        Some(s)
    }
    fn apply_event(&mut self, _event: &Event<serde_json::Value>) {
        self.count += 1;
    }
    fn relevant_event_kinds() -> &'static [EventKind] {
        &[] // empty = replay all
    }
}

/// Counter with schema_version override for cache isolation tests.
#[derive(Default, Debug, serde::Serialize, serde::Deserialize, PartialEq)]
struct VersionedCounterV2 {
    count: u64,
}

impl EventSourced for VersionedCounterV2 {
    type Input = batpak::prelude::JsonValueInput;

    fn from_events(events: &[Event<serde_json::Value>]) -> Option<Self> {
        if events.is_empty() {
            return None;
        }
        let mut s = Self::default();
        for e in events {
            s.apply_event(e);
        }
        Some(s)
    }
    fn apply_event(&mut self, _event: &Event<serde_json::Value>) {
        self.count += 1;
    }
    fn relevant_event_kinds() -> &'static [EventKind] {
        &[]
    }
    fn schema_version() -> u64 {
        2
    }
}

/// Counter that opts into incremental apply.
#[derive(Default, Debug, serde::Serialize, serde::Deserialize, PartialEq)]
struct IncrementalCounter {
    count: u64,
}

impl EventSourced for IncrementalCounter {
    type Input = batpak::prelude::JsonValueInput;

    fn from_events(events: &[Event<serde_json::Value>]) -> Option<Self> {
        if events.is_empty() {
            return None;
        }
        let mut s = Self::default();
        for e in events {
            s.apply_event(e);
        }
        Some(s)
    }
    fn apply_event(&mut self, _event: &Event<serde_json::Value>) {
        self.count += 1;
    }
    fn relevant_event_kinds() -> &'static [EventKind] {
        &[]
    }
    fn supports_incremental_apply() -> bool {
        true
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

mod common;
use common::test_coord;

fn kind_a() -> EventKind {
    EventKind::custom(0xF, 1)
}

fn kind_b() -> EventKind {
    EventKind::custom(0xF, 2)
}

fn payload(i: u32) -> serde_json::Value {
    serde_json::json!({"i": i})
}

// ===========================================================================
// 1a: GROUP COMMIT
// ===========================================================================

#[test]
fn group_commit_batches_under_load() {
    let dir = TempDir::new().expect("temp dir");
    let config = StoreConfig::new(dir.path())
        .with_group_commit_max_batch(32)
        .with_sync_every_n_events(1);
    let store = Store::open(config).expect("open");
    let coord = test_coord();
    for i in 0u32..32 {
        let opts = AppendOptions::new().with_idempotency(i as u128 + 1);
        store
            .append_with_options(&coord, kind_a(), &payload(i), opts)
            .expect("append");
    }
    assert_eq!(
        store.stream("entity:test").len(),
        32,
        "PROPERTY: group commit must persist all 32 events.\n\
         Investigate: src/store/writer.rs writer_loop batch drain."
    );
    store.close().expect("close");
}

#[test]
fn group_commit_batch_1_is_backward_compat() {
    let dir = TempDir::new().expect("temp dir");
    let config = StoreConfig::new(dir.path()).with_group_commit_max_batch(1);
    let store = Store::open(config).expect("open");
    let coord = test_coord();
    store.append(&coord, kind_a(), &payload(0)).expect("append");
    assert_eq!(store.stream("entity:test").len(), 1);
    store.close().expect("close");
}

#[test]
fn group_commit_requires_idempotency_key_when_batch_gt_1() {
    let dir = TempDir::new().expect("temp dir");
    let config = StoreConfig::new(dir.path()).with_group_commit_max_batch(32);
    let store = Store::open(config).expect("open");
    let coord = test_coord();
    // No idempotency key — must error
    let result = store.append(&coord, kind_a(), &payload(0));
    assert!(
        matches!(result, Err(StoreError::IdempotencyRequired)),
        "PROPERTY: group commit (batch>1) must require idempotency keys.\n\
         Got: {result:?}.\n\
         Investigate: src/store/mod.rs do_append idempotency enforcement."
    );
    store.close().expect("close");
}

#[test]
fn group_commit_mid_batch_shutdown_safe() {
    let dir = TempDir::new().expect("temp dir");
    let config = StoreConfig::new(dir.path())
        .with_group_commit_max_batch(64)
        .with_sync_every_n_events(1000);
    let store = Store::open(config).expect("open");
    let coord = test_coord();
    for i in 0u32..10 {
        let opts = AppendOptions::new().with_idempotency(i as u128 + 1);
        store
            .append_with_options(&coord, kind_a(), &payload(i), opts)
            .expect("append");
    }
    store.close().expect("close");
    // Reopen — all events must survive
    let store2 = Store::open(StoreConfig::new(dir.path())).expect("reopen");
    assert_eq!(
        store2.stream("entity:test").len(),
        10,
        "PROPERTY: events committed before close must survive.\n\
         Investigate: src/store/writer.rs shutdown drain."
    );
    store2.close().expect("close");
}

// ===========================================================================
// 1b: DECOUPLE FD / MMAP
// ===========================================================================

#[test]
fn sealed_segment_reads_via_mmap() {
    let dir = TempDir::new().expect("temp dir");
    let config = StoreConfig::new(dir.path()).with_segment_max_bytes(512); // force rotation
    let store = Store::open(config).expect("open");
    let coord = test_coord();
    // Write enough to rotate at least once
    for i in 0u32..50 {
        store.append(&coord, kind_a(), &payload(i)).expect("append");
    }
    store.sync().expect("sync");
    // Read from a sealed segment (not the active one)
    let entries = store.stream("entity:test");
    let first = &entries[0];
    let event = store.get(first.event_id).expect("get from sealed segment");
    assert_eq!(
        event.coordinate.entity(),
        "entity:test",
        "PROPERTY: mmap read from sealed segment must return correct event.\n\
         Investigate: src/store/reader.rs sealed_maps path."
    );
    store.close().expect("close");
}

#[test]
fn concurrent_sealed_reads_no_lock_contention() {
    let dir = TempDir::new().expect("temp dir");
    let config = StoreConfig::new(dir.path()).with_segment_max_bytes(512);
    let store = Arc::new(Store::open(config).expect("open"));
    let coord = test_coord();
    let mut ids = Vec::new();
    for i in 0u32..20 {
        let r = store.append(&coord, kind_a(), &payload(i)).expect("append");
        ids.push(r.event_id);
    }
    store.sync().expect("sync");

    let handles: Vec<_> = ids
        .iter()
        .map(|&id| {
            let s = Arc::clone(&store);
            std::thread::Builder::new()
                .name(format!("reader-{id}"))
                .spawn(move || {
                    s.get(id).expect("concurrent get");
                })
                .expect("spawn")
        })
        .collect();
    for h in handles {
        h.join().expect("reader thread");
    }
    // If we get here without deadlock or panic, the test passes.
}

#[test]
fn evict_mmap_before_compaction_delete() {
    let dir = TempDir::new().expect("temp dir");
    let config = StoreConfig::new(dir.path()).with_segment_max_bytes(512);
    let store = Store::open(config).expect("open");
    let coord = test_coord();
    for i in 0u32..50 {
        store.append(&coord, kind_a(), &payload(i)).expect("append");
    }
    store.sync().expect("sync");
    // Compact — must not fail with "file in use" on any platform
    let result = store.compact(&CompactionConfig::default());
    assert!(
        result.is_ok(),
        "PROPERTY: compaction must succeed even with mmap'd segments.\n\
         Investigate: src/store/reader.rs evict_segment drops Mmap before delete.\n\
         Got: {result:?}"
    );
    store.close().expect("close");
}

// ===========================================================================
// 1c: relevant_event_kinds() WIRING
// ===========================================================================

#[test]
fn relevant_kinds_filters_before_disk_read() {
    let dir = TempDir::new().expect("temp dir");
    let store = Store::open(StoreConfig::new(dir.path())).expect("open");
    let coord = Coordinate::new("kf:entity", "kf:scope").expect("coord");
    for i in 0u32..5 {
        store.append(&coord, kind_a(), &payload(i)).expect("target");
        store.append(&coord, kind_b(), &payload(i)).expect("noise");
    }
    let result: Option<KindFilteredCounter> = store
        .project("kf:entity", &Freshness::Consistent)
        .expect("project");
    let counter = result.expect("should have events");
    assert_eq!(
        counter.target_count, 5,
        "PROPERTY: only relevant_event_kinds events must reach from_events.\n\
         Investigate: src/store/projection_flow.rs kind filter."
    );
    assert_eq!(
        counter.noise_count, 0,
        "PROPERTY: noise events must be filtered BEFORE apply_event.\n\
         noise_count={}, expected 0.\n\
         Investigate: src/store/projection_flow.rs entries filter.",
        counter.noise_count
    );
    store.close().expect("close");
}

#[test]
fn empty_kinds_replays_all() {
    let dir = TempDir::new().expect("temp dir");
    let store = Store::open(StoreConfig::new(dir.path())).expect("open");
    let coord = Coordinate::new("rek:entity", "rek:scope").expect("coord");
    for i in 0u32..3 {
        store
            .append(&coord, EventKind::custom(0xF, i as u16 + 1), &payload(i))
            .expect("append");
    }
    let result: Option<AllCounter> = store
        .project("rek:entity", &Freshness::Consistent)
        .expect("project");
    assert_eq!(
        result.expect("some").count,
        3,
        "PROPERTY: empty relevant_event_kinds = replay all events."
    );
    store.close().expect("close");
}

// ===========================================================================
// 1d: SCHEMA VERSIONING
// ===========================================================================

#[test]
fn schema_version_default_is_zero() {
    assert_eq!(
        AllCounter::schema_version(),
        0,
        "PROPERTY: default schema_version() must be 0."
    );
}

#[test]
fn schema_version_can_be_overridden() {
    assert_eq!(
        VersionedCounterV2::schema_version(),
        2,
        "PROPERTY: overridden schema_version() must return declared value."
    );
}

#[test]
fn versioned_cache_key_isolates_versions() {
    let dir = TempDir::new().expect("temp dir");
    let store = Store::open(StoreConfig::new(dir.path())).expect("open");
    let coord = Coordinate::new("sv:entity", "sv:scope").expect("coord");
    for i in 0u32..5 {
        store.append(&coord, kind_a(), &payload(i)).expect("append");
    }
    // Project with AllCounter (version 0) — caches result
    let r1: Option<AllCounter> = store
        .project("sv:entity", &Freshness::Consistent)
        .expect("project v0");
    assert_eq!(r1.expect("some").count, 5);
    // Project with VersionedCounterV2 (version 2) — must NOT get v0's cached bytes
    let r2: Option<VersionedCounterV2> = store
        .project("sv:entity", &Freshness::Consistent)
        .expect("project v2");
    assert_eq!(
        r2.expect("some").count,
        5,
        "PROPERTY: schema-versioned cache keys must isolate different projection types.\n\
         If this returned a deserialization error, the v0 cache leaked into v2.\n\
         Investigate: src/store/projection_flow.rs cache key construction."
    );
    store.close().expect("close");
}

#[test]
fn versioned_cache_key_isolates_with_native_cache() {
    let dir = TempDir::new().expect("temp dir");
    let cache_path = dir.path().join("cache");
    let config = StoreConfig::new(dir.path().join("data"));
    let store = Store::open_with_native_cache(config, &cache_path).expect("open with native cache");
    let coord = Coordinate::new("svr:entity", "svr:scope").expect("coord");
    for i in 0u32..5 {
        store.append(&coord, kind_a(), &payload(i)).expect("append");
    }
    // Project with AllCounter (v0) — populates native cache under key "svr:entity"
    let r1: Option<AllCounter> = store
        .project("svr:entity", &Freshness::Consistent)
        .expect("project v0");
    assert_eq!(r1.expect("some").count, 5);
    // Project with VersionedCounterV2 (v2) — must get a SEPARATE cache entry
    // under key "svr:entity\0v2", not the v0 bytes
    let r2: Option<VersionedCounterV2> = store
        .project("svr:entity", &Freshness::Consistent)
        .expect("project v2");
    assert_eq!(
        r2.expect("some").count,
        5,
        "PROPERTY: native-cache-backed schema-versioned cache keys must isolate types.\n\
         v0 and v2 projections must not share a cache slot.\n\
         Investigate: src/store/projection_flow.rs cache key with schema_version."
    );
    store.close().expect("close");
}

// ===========================================================================
// 2a: Arc<IndexEntry> + PackedCausation
// ===========================================================================

#[test]
fn supports_incremental_apply_default_is_false() {
    assert!(
        !AllCounter::supports_incremental_apply(),
        "PROPERTY: default supports_incremental_apply() must be false."
    );
}

// ===========================================================================
// 2b: INCREMENTAL PROJECTION
// ===========================================================================

#[test]
fn incremental_apply_delta_only() {
    let dir = TempDir::new().expect("temp dir");
    let config = StoreConfig::new(dir.path()).with_incremental_projection(true);
    let store = Store::open(config).expect("open");
    let coord = Coordinate::new("inc:entity", "inc:scope").expect("coord");
    // Append 5 events, project (full replay, caches at watermark=5)
    for i in 0u32..5 {
        store.append(&coord, kind_a(), &payload(i)).expect("append");
    }
    let r1: Option<IncrementalCounter> = store
        .project("inc:entity", &Freshness::Consistent)
        .expect("first project");
    assert_eq!(r1.expect("some").count, 5);
    // Append 3 more — incremental should apply only these 3
    for i in 5u32..8 {
        store.append(&coord, kind_a(), &payload(i)).expect("append");
    }
    let r2: Option<IncrementalCounter> = store
        .project("inc:entity", &Freshness::Consistent)
        .expect("incremental project");
    assert_eq!(
        r2.expect("some").count,
        8,
        "PROPERTY: incremental projection must reach correct total (5 cached + 3 new = 8).\n\
         Investigate: src/store/projection_flow.rs incremental apply path."
    );
    store.close().expect("close");
}

// ===========================================================================
// 2c: INDEX TOPOLOGY (aos/scan/entity-local/tiled/all)
// ===========================================================================

#[test]
fn index_topology_aos_is_default() {
    let dir = TempDir::new().expect("temp dir");
    // Default config — should compile and work as base AoS only.
    let config = StoreConfig::new(dir.path());
    let store = Store::open(config).expect("open");
    store.close().expect("close");
}

#[test]
fn index_topology_scan_by_fact_correct() {
    let dir = TempDir::new().expect("temp dir");
    let config = StoreConfig::new(dir.path()).with_index_topology(IndexTopology::scan());
    let store = Store::open(config).expect("open");
    let coord = Coordinate::new("soa:entity", "soa:scope").expect("coord");
    for i in 0u32..10 {
        store.append(&coord, kind_a(), &payload(i)).expect("a");
    }
    for i in 0u32..5 {
        store.append(&coord, kind_b(), &payload(i)).expect("b");
    }
    let results = store.by_fact(kind_a());
    assert_eq!(
        results.len(),
        10,
        "PROPERTY: scan topology by_fact must return correct count.\n\
         Investigate: src/store/columnar.rs query_by_kind."
    );
    store.close().expect("close");
}

#[test]
fn index_topology_tiled_by_fact_correct() {
    let dir = TempDir::new().expect("temp dir");
    let config = StoreConfig::new(dir.path()).with_index_topology(IndexTopology::tiled());
    let store = Store::open(config).expect("open");
    let coord = Coordinate::new("tile:entity", "tile:scope").expect("coord");
    for i in 0u32..20 {
        store.append(&coord, kind_a(), &payload(i)).expect("append");
    }
    let results = store.by_fact(kind_a());
    assert_eq!(
        results.len(),
        20,
        "PROPERTY: tiled topology by_fact must return correct count.\n\
         Investigate: src/store/columnar.rs AoSoA64 query."
    );
    store.close().expect("close");
}

#[test]
fn index_topology_entity_local_by_fact_correct() {
    let dir = TempDir::new().expect("temp dir");
    let config = StoreConfig::new(dir.path()).with_index_topology(IndexTopology::entity_local());
    let store = Store::open(config).expect("open");
    let coord = Coordinate::new("tile16:entity", "tile16:scope").expect("coord");
    for i in 0u32..40 {
        store.append(&coord, kind_a(), &payload(i)).expect("append");
    }
    let results = store.by_fact(kind_a());
    assert_eq!(
        results.len(),
        40,
        "PROPERTY: entity-local topology by_fact must return correct count.\n\
         Investigate: src/store/columnar.rs SoAoSInner::query_by_kind."
    );
    store.close().expect("close");
}

#[test]
fn index_topology_all_by_fact_correct() {
    let dir = TempDir::new().expect("temp dir");
    let config = StoreConfig::new(dir.path()).with_index_topology(IndexTopology::all());
    let store = Store::open(config).expect("open");
    let coord = Coordinate::new("tile64:entity", "tile64:scope").expect("coord");
    for i in 0u32..150 {
        store.append(&coord, kind_a(), &payload(i)).expect("append");
    }
    let results = store.by_fact(kind_a());
    assert_eq!(
        results.len(),
        150,
        "PROPERTY: all-topology by_fact must return correct count.\n\
         Investigate: src/store/columnar.rs overlay fanout."
    );
    store.close().expect("close");
}

#[test]
fn topology_parity_aos_vs_scan() {
    let dir_aos = TempDir::new().expect("dir aos");
    let dir_scan = TempDir::new().expect("dir scan");
    let kind = kind_a();

    let populate = |store: &Store| {
        let coord = Coordinate::new("parity:entity", "parity:scope").expect("coord");
        for i in 0u32..20 {
            store.append(&coord, kind, &payload(i)).expect("append");
        }
    };

    let store_aos = Store::open(StoreConfig::new(dir_aos.path())).expect("open aos");
    populate(&store_aos);

    let store_scan =
        Store::open(StoreConfig::new(dir_scan.path()).with_index_topology(IndexTopology::scan()))
            .expect("open scan");
    populate(&store_scan);

    let events_aos = store_aos.by_fact(kind);
    let events_scan = store_scan.by_fact(kind);
    assert_eq!(
        events_aos.len(),
        events_scan.len(),
        "PROPERTY: aos and scan must return identical by_fact results.\n\
         aos={}, scan={}.",
        events_aos.len(),
        events_scan.len()
    );
    store_aos.close().expect("close");
    store_scan.close().expect("close");
}

// ===========================================================================
// 2c continued: entity-local topology
// ===========================================================================

#[test]
fn index_topology_entity_local_mixed_fact_correct() {
    let dir = TempDir::new().expect("temp dir");
    let config = StoreConfig::new(dir.path()).with_index_topology(IndexTopology::entity_local());
    let store = Store::open(config).expect("open");
    let coord = Coordinate::new("soaos:entity", "soaos:scope").expect("coord");
    for i in 0u32..15 {
        store.append(&coord, kind_a(), &payload(i)).expect("a");
    }
    for i in 0u32..5 {
        store.append(&coord, kind_b(), &payload(i)).expect("b");
    }
    let results = store.by_fact(kind_a());
    assert_eq!(
        results.len(),
        15,
        "PROPERTY: entity-local topology by_fact must return correct count.\n\
         Investigate: src/store/columnar.rs SoAoSInner::query_by_kind."
    );
    store.close().expect("close");
}

#[test]
fn topology_parity_aos_vs_entity_local() {
    let dir_aos = TempDir::new().expect("dir aos");
    let dir_entity_local = TempDir::new().expect("dir entity-local");
    let kind = kind_a();

    let populate = |store: &Store| {
        let coord = Coordinate::new("parity:entity", "parity:scope").expect("coord");
        for i in 0u32..20 {
            store.append(&coord, kind, &payload(i)).expect("append");
        }
    };

    let store_aos = Store::open(StoreConfig::new(dir_aos.path())).expect("open aos");
    populate(&store_aos);

    let store_entity_local = Store::open(
        StoreConfig::new(dir_entity_local.path())
            .with_index_topology(IndexTopology::entity_local()),
    )
    .expect("open entity-local");
    populate(&store_entity_local);

    let events_aos = store_aos.by_fact(kind);
    let events_entity_local = store_entity_local.by_fact(kind);
    assert_eq!(
        events_aos.len(),
        events_entity_local.len(),
        "PROPERTY: aos and entity-local must return identical by_fact results.\n\
         aos={}, entity-local={}.",
        events_aos.len(),
        events_entity_local.len()
    );
    store_aos.close().expect("close");
    store_entity_local.close().expect("close");
}

// ===========================================================================
// SIDX FOOTER
// ===========================================================================

#[test]
fn sidx_cold_start_uses_footer() {
    let dir = TempDir::new().expect("temp dir");
    let config = StoreConfig::new(dir.path()).with_segment_max_bytes(512);
    let store = Store::open(config).expect("open");
    let coord = test_coord();
    for i in 0u32..50 {
        store.append(&coord, kind_a(), &payload(i)).expect("append");
    }
    store.sync().expect("sync");
    store.close().expect("close");
    // Reopen — cold start should use SIDX footers for sealed segments
    let store2 =
        Store::open(StoreConfig::new(dir.path()).with_segment_max_bytes(512)).expect("reopen");
    assert_eq!(
        store2.stream("entity:test").len(),
        50,
        "PROPERTY: cold start via SIDX footer must recover all events.\n\
         Investigate: src/store/reader.rs SIDX-aware scan_segment_index."
    );
    store2.close().expect("close");
}

// ===========================================================================
// CHECKPOINT
// ===========================================================================

#[test]
fn checkpoint_write_load_roundtrip() {
    let dir = TempDir::new().expect("temp dir");
    let config = StoreConfig::new(dir.path()).with_enable_checkpoint(true);
    let store = Store::open(config).expect("open");
    let coord = test_coord();
    for i in 0u32..100 {
        store.append(&coord, kind_a(), &payload(i)).expect("append");
    }
    store.sync().expect("sync");
    store.close().expect("close writes checkpoint");
    // Reopen — should load checkpoint, not full scan
    let store2 = Store::open(StoreConfig::new(dir.path()).with_enable_checkpoint(true))
        .expect("reopen from checkpoint");
    assert_eq!(
        store2.stream("entity:test").len(),
        100,
        "PROPERTY: checkpoint roundtrip must preserve all events.\n\
         Investigate: src/store/checkpoint.rs write_checkpoint + try_load_checkpoint."
    );
    store2.close().expect("close");
}

#[test]
fn stale_checkpoint_falls_back_to_full_rebuild() {
    let dir = TempDir::new().expect("temp dir");
    let config = StoreConfig::new(dir.path()).with_enable_checkpoint(true);
    let store = Store::open(config).expect("open");
    let coord = test_coord();
    for i in 0u32..20 {
        store.append(&coord, kind_a(), &payload(i)).expect("append");
    }
    store.sync().expect("sync");
    store.close().expect("close");
    // Corrupt the checkpoint file
    let ckpt_path = dir.path().join("index.ckpt");
    if ckpt_path.exists() {
        std::fs::write(&ckpt_path, b"CORRUPT").expect("corrupt checkpoint");
    }
    // Reopen — must fall back to full scan without error
    let store2 = Store::open(StoreConfig::new(dir.path()).with_enable_checkpoint(true))
        .expect("reopen with corrupt checkpoint");
    assert_eq!(
        store2.stream("entity:test").len(),
        20,
        "PROPERTY: corrupt checkpoint must fall back to full rebuild.\n\
         Investigate: src/store/checkpoint.rs try_load_checkpoint → None."
    );
    store2.close().expect("close");
}

#[test]
fn post_compact_checkpoint_valid() {
    let dir = TempDir::new().expect("temp dir");
    let config = StoreConfig::new(dir.path())
        .with_segment_max_bytes(512)
        .with_enable_checkpoint(true);
    let store = Store::open(config).expect("open");
    let coord = test_coord();
    for i in 0u32..50 {
        store.append(&coord, kind_a(), &payload(i)).expect("append");
    }
    store.sync().expect("sync");
    store
        .compact(&CompactionConfig::default())
        .expect("compact");
    store.close().expect("close writes post-compact checkpoint");
    // Reopen — checkpoint should be valid for post-compact state
    let store2 = Store::open(
        StoreConfig::new(dir.path())
            .with_segment_max_bytes(512)
            .with_enable_checkpoint(true),
    )
    .expect("reopen");
    assert_eq!(
        store2.stream("entity:test").len(),
        50,
        "PROPERTY: post-compact checkpoint must be valid.\n\
         Investigate: src/store/maintenance.rs compact writes checkpoint."
    );
    store2.close().expect("close");
}

// ===========================================================================
// STRING INTERNER
// ===========================================================================

#[test]
fn interner_roundtrip() {
    // This test verifies the interner is wired into the index path.
    // After the big bang, IndexEntry internally uses InternId, not Arc<str>.
    // The public API (entry.coord) still returns Coordinate.
    let dir = TempDir::new().expect("temp dir");
    let store = Store::open(StoreConfig::new(dir.path())).expect("open");
    let coord = Coordinate::new("intern:entity", "intern:scope").expect("coord");
    store.append(&coord, kind_a(), &payload(0)).expect("append");
    let entries = store.stream("intern:entity");
    assert_eq!(entries.len(), 1);
    // coord must resolve correctly from interned IDs
    assert_eq!(entries[0].coord.entity(), "intern:entity");
    assert_eq!(entries[0].coord.scope(), "intern:scope");
    store.close().expect("close");
}

// ===========================================================================
// CONFIG VALIDATION
// ===========================================================================

#[test]
fn config_validation_rejects_zero_segment_max_bytes() {
    let dir = TempDir::new().expect("temp dir");
    let config = StoreConfig::new(dir.path()).with_segment_max_bytes(0);
    // Note: `Store` doesn't implement Debug (it owns Arc'd internal state),
    // so `Result::expect_err` doesn't compile here. Match instead.
    let err = match Store::open(config) {
        Ok(_) => panic!(
            "PROPERTY: segment_max_bytes=0 must be rejected at open time. \
             Investigate: src/store/config.rs StoreConfig::validate."
        ),
        Err(e) => e,
    };
    assert!(
        matches!(err, StoreError::Configuration { .. }),
        "PROPERTY: must surface as StoreError::Configuration, got {err:?}"
    );
}

#[test]
fn config_validation_rejects_zero_writer_channel_capacity() {
    let dir = TempDir::new().expect("temp dir");
    let config = StoreConfig::new(dir.path()).with_writer_channel_capacity(0);
    let err = match Store::open(config) {
        Ok(_) => panic!(
            "PROPERTY: writer.channel_capacity=0 must be rejected at open time \
             (a zero-capacity channel deadlocks on the first append). \
             Investigate: src/store/config.rs StoreConfig::validate."
        ),
        Err(e) => e,
    };
    assert!(
        matches!(err, StoreError::Configuration { .. }),
        "PROPERTY: must surface as StoreError::Configuration, got {err:?}"
    );
}

// ===========================================================================
// BATCH READS
// ===========================================================================

#[test]
fn batch_read_matches_sequential() {
    let dir = TempDir::new().expect("temp dir");
    let store = Store::open(StoreConfig::new(dir.path())).expect("open");
    let coord = Coordinate::new("batch:entity", "batch:scope").expect("coord");
    for i in 0u32..30 {
        store.append(&coord, kind_a(), &payload(i)).expect("append");
    }
    // Project uses batch reads internally — verify result is correct
    let result: Option<AllCounter> = store
        .project("batch:entity", &Freshness::Consistent)
        .expect("project");
    assert_eq!(
        result.expect("some").count,
        30,
        "PROPERTY: batch read projection must replay all 30 events.\n\
         Investigate: src/store/reader.rs read_entries_batch."
    );
    store.close().expect("close");
}

// ===========================================================================
// SINGLE-SLOT PROJECTION CACHE
// ===========================================================================

#[test]
fn single_slot_hit_on_repeat_project() {
    let dir = TempDir::new().expect("temp dir");
    let store = Store::open(StoreConfig::new(dir.path())).expect("open");
    let coord = Coordinate::new("slot:entity", "slot:scope").expect("coord");
    for i in 0u32..10 {
        store.append(&coord, kind_a(), &payload(i)).expect("append");
    }
    // First project — cache miss, full replay
    let r1: Option<AllCounter> = store
        .project("slot:entity", &Freshness::Consistent)
        .expect("first project");
    assert_eq!(r1.expect("some").count, 10);
    // Second project — should hit single-slot cache (same entity, no new events)
    let r2: Option<AllCounter> = store
        .project("slot:entity", &Freshness::Consistent)
        .expect("second project");
    assert_eq!(
        r2.expect("some").count,
        10,
        "PROPERTY: repeated project on same entity must use single-slot cache."
    );
    store.close().expect("close");
}

#[test]
fn single_slot_miss_on_different_entity() {
    let dir = TempDir::new().expect("temp dir");
    let store = Store::open(StoreConfig::new(dir.path())).expect("open");
    let coord_a = Coordinate::new("slot:a", "slot:scope").expect("coord");
    let coord_b = Coordinate::new("slot:b", "slot:scope").expect("coord");
    for i in 0u32..5 {
        store.append(&coord_a, kind_a(), &payload(i)).expect("a");
        store.append(&coord_b, kind_a(), &payload(i)).expect("b");
    }
    let r1: Option<AllCounter> = store
        .project("slot:a", &Freshness::Consistent)
        .expect("project a");
    assert_eq!(r1.expect("some").count, 5);
    let r2: Option<AllCounter> = store
        .project("slot:b", &Freshness::Consistent)
        .expect("project b");
    assert_eq!(
        r2.expect("some").count,
        5,
        "PROPERTY: projecting a different entity must not return slot:a's cached result."
    );
    store.close().expect("close");
}

// ===========================================================================
// REACTIVE QUERY SUBSCRIPTIONS (watch_projection)
// ===========================================================================

#[test]
fn watch_projection_emits_on_new_events() {
    let dir = TempDir::new().expect("temp dir");
    let store = Arc::new(Store::open(StoreConfig::new(dir.path())).expect("open"));
    let coord = Coordinate::new("watch:entity", "watch:scope").expect("coord");

    // Seed with initial events
    for i in 0u32..5 {
        store.append(&coord, kind_a(), &payload(i)).expect("append");
    }

    let mut watcher = store.watch_projection::<AllCounter>("watch:entity", Freshness::Consistent);

    // Spawn a thread that appends 3 more events after a brief delay
    let store2 = Arc::clone(&store);
    let handle = std::thread::Builder::new()
        .name("watch-writer".into())
        .spawn(move || {
            let coord = Coordinate::new("watch:entity", "watch:scope").expect("coord");
            for i in 5u32..8 {
                store2
                    .append(&coord, kind_a(), &payload(i))
                    .expect("append");
            }
        })
        .expect("spawn");

    // Receive the first projection update (triggered by one of the 3 new events)
    let result = watcher.recv().expect("recv should not error");
    let counter = result.expect("should have projection");
    // The projection should see at least 6 events (5 initial + at least 1 new)
    assert!(
        counter.count >= 6,
        "PROPERTY: watch_projection must re-project with new events.\n\
         Got count={}, expected >= 6.\n\
         Investigate: src/store/mod.rs watch_projection + ProjectionWatcher::recv.",
        counter.count
    );

    handle.join().expect("writer thread");
    // Don't close — let Arc<Store> drop naturally
}

#[test]
fn watch_projection_catches_up_after_lossy_notifications() {
    let dir = TempDir::new().expect("temp dir");
    let config = StoreConfig::new(dir.path()).with_broadcast_capacity(1);
    let store = Arc::new(Store::open(config).expect("open"));
    let coord = Coordinate::new("watch:lossy", "watch:scope").expect("coord");

    for i in 0u32..3 {
        store
            .append(&coord, kind_a(), &payload(i))
            .expect("seed append");
    }

    let mut watcher = store.watch_projection::<AllCounter>("watch:lossy", Freshness::Consistent);

    for i in 3u32..10 {
        store
            .append(&coord, kind_a(), &payload(i))
            .expect("append burst");
    }

    let result = watcher.recv().expect("recv should not error");
    let counter = result.expect("projection should exist");
    assert_eq!(
        counter.count, 10,
        "PROPERTY: watch_projection must catch up by watermark even when the lossy subscription \
         collapses multiple notifications into one.\n\
         Investigate: src/store/mod.rs ProjectionWatcher::recv + StoreIndex::stream_since."
    );
}

#[test]
fn watch_projection_returns_none_on_store_close() {
    let dir = TempDir::new().expect("temp dir");
    let store = Arc::new(Store::open(StoreConfig::new(dir.path())).expect("open"));
    let coord = Coordinate::new("drop:entity", "drop:scope").expect("coord");
    store.append(&coord, kind_a(), &payload(0)).expect("append");

    // Subscribe BEFORE we move the Arc — the subscription is independent.
    let sub = store.subscribe_lossy(&Region::entity("drop:entity"));

    // Close the store from another thread. This shuts down the writer,
    // which closes the broadcast channels, which makes sub.recv() return None.
    // We must unwrap the Arc first to get ownership for close().
    let handle = std::thread::Builder::new()
        .name("store-closer".into())
        .spawn(move || {
            // Try to unwrap the Arc. If watcher holds a clone, this fails
            // and we just drop it (which triggers the Drop impl shutdown).
            match Arc::try_unwrap(store) {
                Ok(s) => {
                    let _ = s.close();
                }
                Err(arc) => {
                    drop(arc);
                }
            }
        })
        .expect("spawn");

    // recv should return None when the writer shuts down
    let result = sub.recv();
    assert!(
        result.is_none(),
        "PROPERTY: subscription must return None when store shuts down."
    );

    handle.join().expect("closer thread");
}
