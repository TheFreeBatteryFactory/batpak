use batpak::prelude::*;

#[test]
fn store_config_builder_methods_are_chainable() {
    let clock: std::sync::Arc<dyn Fn() -> i64 + Send + Sync> = std::sync::Arc::new(|| 42);
    let config = StoreConfig::new("./batpak-data")
        .with_segment_max_bytes(1024)
        .with_sync_every_n_events(7)
        .with_fd_budget(9)
        .with_writer_channel_capacity(10)
        .with_broadcast_capacity(11)
        .with_cache_map_size_bytes(12)
        .with_restart_policy(RestartPolicy::Bounded {
            max_restarts: 2,
            within_ms: 30_000,
        })
        .with_shutdown_drain_limit(13)
        .with_writer_stack_size(Some(14))
        .with_clock(Some(clock))
        .with_sync_mode(SyncMode::SyncData);

    assert_eq!(config.segment_max_bytes, 1024);
    assert_eq!(config.sync_every_n_events, 7);
    assert_eq!(config.fd_budget, 9);
    assert_eq!(config.writer_channel_capacity, 10);
    assert_eq!(config.broadcast_capacity, 11);
    assert_eq!(config.cache_map_size_bytes, 12);
    assert!(matches!(
        config.restart_policy,
        RestartPolicy::Bounded {
            max_restarts: 2,
            within_ms: 30_000
        }
    ));
    assert_eq!(config.shutdown_drain_limit, 13);
    assert_eq!(config.writer_stack_size, Some(14));
    assert!(config.clock.is_some());
    assert!(matches!(config.sync_mode, SyncMode::SyncData));
}

#[cfg(feature = "redb")]
#[test]
fn open_with_redb_cache_is_available_for_common_setup() {
    let data_dir = tempfile::tempdir().expect("data dir");
    let cache_dir = tempfile::tempdir().expect("cache dir");
    let store = Store::open_with_redb_cache(
        StoreConfig::new(data_dir.path()),
        cache_dir.path().join("projection.redb"),
    )
    .expect("open store with redb cache");
    let coord = Coordinate::new("entity:redb", "scope:test").expect("coord");
    let kind = EventKind::custom(0xF, 1);
    let receipt = store
        .append(&coord, kind, &serde_json::json!({"hello": "redb"}))
        .expect("append");
    assert!(receipt.event_id != 0);
}

#[cfg(feature = "lmdb")]
#[test]
fn open_with_lmdb_cache_is_available_for_common_setup() {
    let data_dir = tempfile::tempdir().expect("data dir");
    let cache_dir = tempfile::tempdir().expect("cache dir");
    let config = StoreConfig::new(data_dir.path()).with_cache_map_size_bytes(4 * 1024 * 1024);
    let store =
        Store::open_with_lmdb_cache(config, cache_dir.path()).expect("open store with lmdb");
    let coord = Coordinate::new("entity:lmdb", "scope:test").expect("coord");
    let kind = EventKind::custom(0xF, 1);
    let receipt = store
        .append(&coord, kind, &serde_json::json!({"hello": "lmdb"}))
        .expect("append");
    assert!(receipt.event_id != 0);
}
