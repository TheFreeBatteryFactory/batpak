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

/// Snapshot of writer mailbox pressure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[must_use]
pub struct WriterPressure {
    /// Number of queued commands currently waiting in the writer mailbox.
    pub queue_len: usize,
    /// Configured bounded capacity of the writer mailbox.
    pub capacity: usize,
}

impl WriterPressure {
    /// Fraction of mailbox capacity currently in use.
    pub fn utilization(&self) -> f64 {
        if self.capacity == 0 {
            return 0.0;
        }
        self.queue_len as f64 / self.capacity as f64
    }

    /// Number of free command slots remaining before the mailbox is full.
    pub fn headroom(&self) -> usize {
        self.capacity.saturating_sub(self.queue_len)
    }

    /// True when the mailbox has no queued commands.
    pub fn is_idle(&self) -> bool {
        self.queue_len == 0
    }
}

/// Detailed diagnostic snapshot of the store's internal configuration and state.
#[derive(Clone, Debug)]
#[must_use]
pub struct StoreDiagnostics {
    /// Total number of events currently held in the in-memory index.
    pub event_count: usize,
    /// Current value of the global monotonic sequence counter (allocator).
    pub global_sequence: u64,
    /// Current visibility watermark (exclusive upper bound).
    /// Entries with `global_sequence < visible_sequence` are returned by read methods.
    pub visible_sequence: u64,
    /// Filesystem path to the directory containing segment files.
    pub data_dir: PathBuf,
    /// Maximum segment file size in bytes before rotation.
    pub segment_max_bytes: u64,
    /// Maximum number of concurrently open segment file descriptors.
    pub fd_budget: usize,
    /// Writer thread restart policy used on panic.
    pub restart_policy: RestartPolicy,
    /// Current writer mailbox pressure snapshot.
    pub writer_pressure: WriterPressure,
    /// Active scan topology name (`AoS` or `MultiView`).
    pub index_layout: &'static str,
    /// Number of tiles in the columnar index (0 for non-tiled layouts).
    pub tile_count: usize,
}
