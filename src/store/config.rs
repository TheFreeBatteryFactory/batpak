use crate::store::RestartPolicy;
use std::path::PathBuf;
use std::sync::Arc;

/// Sync strategy for segment fsync.
#[derive(Clone, Debug, Default)]
pub enum SyncMode {
    /// sync_all: syncs data + metadata (safest, slower)
    #[default]
    SyncAll,
    /// sync_data: syncs data only (faster, sufficient for most use cases)
    SyncData,
}

/// StoreConfig: all settings for a Store instance.
/// No Default — callers must provide data_dir via `StoreConfig::new(path)`.
/// Manual Clone and Debug impls because `clock` field is `Arc<dyn Fn>`.
pub struct StoreConfig {
    /// Directory where segment files (.fbat) are stored.
    pub data_dir: PathBuf,
    /// Maximum bytes per segment file before rotation.
    pub segment_max_bytes: u64,
    /// Number of events between periodic fsyncs.
    pub sync_every_n_events: u32,
    /// Maximum number of open segment file descriptors.
    pub fd_budget: usize,
    /// Capacity of the flume channel between callers and the writer thread.
    pub writer_channel_capacity: usize,
    /// Capacity of each subscriber's broadcast channel.
    pub broadcast_capacity: usize,
    /// Maximum size in bytes of the LMDB cache map.
    pub cache_map_size_bytes: usize,
    /// Writer auto-restart policy on panic. `Once` allows 1 restart, `Bounded`
    /// allows N restarts within a time window. See: writer.rs writer_thread_main().
    pub restart_policy: RestartPolicy,
    /// Maximum number of queued append commands drained during shutdown.
    pub shutdown_drain_limit: usize,
    /// Optional writer thread stack size. None = OS default (~8MB on Linux).
    pub writer_stack_size: Option<usize>,
    /// Injectable clock for deterministic testing. Returns microseconds since epoch.
    /// None = std::time::SystemTime::now() (production default).
    pub clock: Option<Arc<dyn Fn() -> i64 + Send + Sync>>,
    /// Sync mode: SyncAll (data+metadata, default) or SyncData (data only, faster).
    pub sync_mode: SyncMode,
}

impl StoreConfig {
    /// Create a StoreConfig with required data_dir and sensible defaults.
    /// All numeric defaults are documented. Override fields after construction
    /// to tune for your deployment (embedded, server, CLI).
    pub fn new(data_dir: impl Into<PathBuf>) -> Self {
        Self {
            data_dir: data_dir.into(),
            segment_max_bytes: 256 * 1024 * 1024,
            sync_every_n_events: 1000,
            fd_budget: 64,
            writer_channel_capacity: 4096,
            broadcast_capacity: 8192,
            cache_map_size_bytes: 64 * 1024 * 1024,
            restart_policy: RestartPolicy::default(),
            shutdown_drain_limit: 1024,
            writer_stack_size: None,
            clock: None,
            sync_mode: SyncMode::default(),
        }
    }

    /// Set the maximum segment file size in bytes before rotation.
    pub fn with_segment_max_bytes(mut self, segment_max_bytes: u64) -> Self {
        self.segment_max_bytes = segment_max_bytes;
        self
    }

    /// Set how many events are written between periodic fsyncs.
    pub fn with_sync_every_n_events(mut self, sync_every_n_events: u32) -> Self {
        self.sync_every_n_events = sync_every_n_events;
        self
    }

    /// Set the maximum number of concurrently open segment file descriptors.
    pub fn with_fd_budget(mut self, fd_budget: usize) -> Self {
        self.fd_budget = fd_budget;
        self
    }

    /// Set the capacity of the writer command channel.
    pub fn with_writer_channel_capacity(mut self, writer_channel_capacity: usize) -> Self {
        self.writer_channel_capacity = writer_channel_capacity;
        self
    }

    /// Set the per-subscriber broadcast channel capacity.
    pub fn with_broadcast_capacity(mut self, broadcast_capacity: usize) -> Self {
        self.broadcast_capacity = broadcast_capacity;
        self
    }

    /// Set the LMDB cache map size in bytes.
    pub fn with_cache_map_size_bytes(mut self, cache_map_size_bytes: usize) -> Self {
        self.cache_map_size_bytes = cache_map_size_bytes;
        self
    }

    /// Set the writer thread restart policy on panic.
    pub fn with_restart_policy(mut self, restart_policy: RestartPolicy) -> Self {
        self.restart_policy = restart_policy;
        self
    }

    /// Set how many pending appends the writer drains before shutting down.
    pub fn with_shutdown_drain_limit(mut self, shutdown_drain_limit: usize) -> Self {
        self.shutdown_drain_limit = shutdown_drain_limit;
        self
    }

    /// Set an explicit stack size for the writer thread; `None` uses the OS default.
    pub fn with_writer_stack_size(mut self, writer_stack_size: Option<usize>) -> Self {
        self.writer_stack_size = writer_stack_size;
        self
    }

    /// Override the clock with a custom function returning microseconds since epoch (for testing).
    pub fn with_clock(mut self, clock: Option<Arc<dyn Fn() -> i64 + Send + Sync>>) -> Self {
        self.clock = clock;
        self
    }

    /// Set the fsync strategy used after writes.
    pub fn with_sync_mode(mut self, sync_mode: SyncMode) -> Self {
        self.sync_mode = sync_mode;
        self
    }

    /// Get current timestamp in microseconds, using the injectable clock if set.
    pub(crate) fn now_us(&self) -> i64 {
        match &self.clock {
            Some(f) => f(),
            None => now_us(),
        }
    }
}

impl Clone for StoreConfig {
    fn clone(&self) -> Self {
        Self {
            data_dir: self.data_dir.clone(),
            segment_max_bytes: self.segment_max_bytes,
            sync_every_n_events: self.sync_every_n_events,
            fd_budget: self.fd_budget,
            writer_channel_capacity: self.writer_channel_capacity,
            broadcast_capacity: self.broadcast_capacity,
            cache_map_size_bytes: self.cache_map_size_bytes,
            restart_policy: self.restart_policy.clone(),
            shutdown_drain_limit: self.shutdown_drain_limit,
            writer_stack_size: self.writer_stack_size,
            clock: self.clock.clone(),
            sync_mode: self.sync_mode.clone(),
        }
    }
}

impl std::fmt::Debug for StoreConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StoreConfig")
            .field("data_dir", &self.data_dir)
            .field("segment_max_bytes", &self.segment_max_bytes)
            .field("sync_every_n_events", &self.sync_every_n_events)
            .field("fd_budget", &self.fd_budget)
            .field("writer_channel_capacity", &self.writer_channel_capacity)
            .field("broadcast_capacity", &self.broadcast_capacity)
            .field("cache_map_size_bytes", &self.cache_map_size_bytes)
            .field("restart_policy", &self.restart_policy)
            .field("shutdown_drain_limit", &self.shutdown_drain_limit)
            .field("writer_stack_size", &self.writer_stack_size)
            .field("clock", &self.clock.as_ref().map(|_| "<fn>"))
            .field("sync_mode", &self.sync_mode)
            .finish()
    }
}

pub(crate) fn now_us() -> i64 {
    // Unix epoch micros fit in i64 for any practical lifetime of this project.
    #[allow(clippy::cast_possible_truncation)]
    {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_micros() as i64
    }
}
