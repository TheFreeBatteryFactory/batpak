use std::path::PathBuf;

use crate::store::RestartPolicy;

#[derive(Clone, Debug)]
#[must_use]
pub struct StoreStats {
    pub event_count: usize,
    pub global_sequence: u64,
}

#[derive(Clone, Debug)]
#[must_use]
pub struct StoreDiagnostics {
    pub event_count: usize,
    pub global_sequence: u64,
    pub data_dir: PathBuf,
    pub segment_max_bytes: u64,
    pub fd_budget: usize,
    pub restart_policy: RestartPolicy,
}
