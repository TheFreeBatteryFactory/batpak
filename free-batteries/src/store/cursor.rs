use crate::coordinate::Region;
use crate::store::index::{StoreIndex, IndexEntry};
use std::sync::Arc;

/// Cursor: pull-based event consumption with guaranteed delivery.
/// Reads from index, not channels. Cannot lose events.
/// [SPEC:src/store/cursor.rs]
pub struct Cursor {
    region: Region,
    position: u64,      // tracks global_sequence — next poll starts after this
    index: Arc<StoreIndex>,
}

impl Cursor {
    pub(crate) fn new(region: Region, index: Arc<StoreIndex>) -> Self {
        Self { region, position: 0, index }
    }

    /// Poll for the next matching event after our current position.
    pub fn poll(&mut self) -> Option<IndexEntry> {
        /// Query the index for events matching our region with global_sequence > self.position.
        /// Return the first match, advance position.
        let results = self.index.query(&self.region);
        for entry in results {
            if entry.global_sequence > self.position {
                self.position = entry.global_sequence;
                return Some(entry);
            }
        }
        None
    }

    /// Poll for up to max matching events.
    pub fn poll_batch(&mut self, max: usize) -> Vec<IndexEntry> {
        let mut batch = Vec::with_capacity(max);
        let results = self.index.query(&self.region);
        for entry in results {
            if entry.global_sequence > self.position {
                self.position = entry.global_sequence;
                batch.push(entry);
                if batch.len() >= max { break; }
            }
        }
        batch
    }
}
