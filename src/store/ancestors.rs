use crate::event::StoredEvent;
use crate::store::Store;

#[cfg(not(feature = "blake3"))]
#[path = "ancestors_clock.rs"]
mod ancestors_clock;
#[cfg(feature = "blake3")]
#[path = "ancestors_hash.rs"]
mod ancestors_hash;

pub(crate) fn walk_ancestors<State>(
    store: &Store<State>,
    event_id: u128,
    limit: usize,
) -> Vec<StoredEvent<serde_json::Value>> {
    #[cfg(feature = "blake3")]
    {
        ancestors_hash::walk_ancestors_by_hash(store, event_id, limit)
    }

    #[cfg(not(feature = "blake3"))]
    {
        ancestors_clock::walk_ancestors_by_clock(store, event_id, limit)
    }
}
