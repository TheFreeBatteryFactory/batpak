use crate::event::StoredEvent;
use crate::store::{DiskPos, StoreError};

/// AppendReceipt: proof an event was persisted.
#[derive(Clone, Debug)]
pub struct AppendReceipt {
    pub event_id: u128,
    pub sequence: u64,
    pub disk_pos: DiskPos,
}

/// AppendOptions: CAS, idempotency, custom correlation/causation.
/// [SPEC:src/store/mod.rs — AppendOptions]
#[derive(Clone, Copy, Debug, Default)]
pub struct AppendOptions {
    pub expected_sequence: Option<u32>,
    pub idempotency_key: Option<u128>,
    pub correlation_id: Option<u128>,
    pub causation_id: Option<u128>,
    /// EventHeader flags (FLAG_REQUIRES_ACK, FLAG_TRANSACTIONAL, FLAG_REPLAY).
    /// Default: 0 (no flags). [SPEC:src/event/header.rs — Flag bit constants]
    pub flags: u8,
}

impl AppendOptions {
    /// Create new AppendOptions with all defaults.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set expected sequence for compare-and-swap (CAS) check.
    pub fn with_cas(mut self, seq: u32) -> Self {
        self.expected_sequence = Some(seq);
        self
    }

    /// Set idempotency key. Duplicate appends with the same key return the original receipt.
    pub fn with_idempotency(mut self, key: u128) -> Self {
        self.idempotency_key = Some(key);
        self
    }

    /// Set EventHeader flags (bitwise OR of FLAG_REQUIRES_ACK, FLAG_TRANSACTIONAL, FLAG_REPLAY).
    pub fn with_flags(mut self, flags: u8) -> Self {
        self.flags = flags;
        self
    }

    /// Set custom correlation ID.
    pub fn with_correlation(mut self, id: u128) -> Self {
        self.correlation_id = Some(id);
        self
    }

    /// Set custom causation ID.
    pub fn with_causation(mut self, id: u128) -> Self {
        self.causation_id = Some(id);
        self
    }
}

/// Predicate for filtering events during compaction. Returns true to keep, false to drop.
pub type RetentionPredicate = Box<dyn Fn(&StoredEvent<serde_json::Value>) -> bool + Send>;

/// CompactionStrategy: how compact() handles events during segment merging.
#[non_exhaustive]
pub enum CompactionStrategy {
    /// Merge sealed segments into one. No events removed.
    Merge,
    /// Merge + drop events failing the retention predicate.
    /// Dropped events are permanently lost.
    Retention(RetentionPredicate),
    /// Merge + write tombstone markers for dropped events.
    /// Downstream consumers can detect deletions.
    Tombstone(RetentionPredicate),
}

/// CompactionConfig: controls compact() behavior.
pub struct CompactionConfig {
    /// Strategy for handling events during compaction.
    pub strategy: CompactionStrategy,
    /// Minimum number of sealed segments before compaction runs.
    /// Below this threshold, compact() returns early.
    pub min_segments: usize,
}

impl Default for CompactionConfig {
    fn default() -> Self {
        Self {
            strategy: CompactionStrategy::Merge,
            min_segments: 2,
        }
    }
}

/// Validate payload length fits in u32. Prevents silent truncation
/// when serialized payloads exceed 4GB (unlikely but possible with
/// pathological inputs or corrupted serialization).
pub(crate) fn checked_payload_len(payload_bytes: &[u8]) -> Result<u32, StoreError> {
    u32::try_from(payload_bytes.len()).map_err(|_| {
        StoreError::Serialization(format!(
            "payload size {} exceeds u32::MAX (4GB limit)",
            payload_bytes.len()
        ))
    })
}
