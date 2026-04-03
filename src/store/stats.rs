use std::path::PathBuf;

use crate::store::RestartPolicy;

/// Lightweight runtime statistics snapshot for the store.
#[derive(Clone, Debug)]
#[must_use]
pub struct StoreStats {
    /// Total number of events currently held in the in-memory index.
    pub event_count: usize,
    /// Current value of the global monotonic sequence counter.
    pub global_sequence: u64,
}

/// Detailed diagnostic snapshot of the store's internal configuration and state.
#[derive(Clone, Debug)]
#[must_use]
pub struct StoreDiagnostics {
    /// Total number of events currently held in the in-memory index.
    pub event_count: usize,
    /// Current value of the global monotonic sequence counter.
    pub global_sequence: u64,
    /// Filesystem path to the directory containing segment files.
    pub data_dir: PathBuf,
    /// Maximum segment file size in bytes before rotation.
    pub segment_max_bytes: u64,
    /// Maximum number of concurrently open segment file descriptors.
    pub fd_budget: usize,
    /// Writer thread restart policy used on panic.
    pub restart_policy: RestartPolicy,
}
