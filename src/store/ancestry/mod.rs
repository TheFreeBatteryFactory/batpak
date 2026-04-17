use crate::event::StoredEvent;
use crate::store::IndexEntry;
use crate::store::Store;

#[cfg(not(feature = "blake3"))]
mod by_clock;
#[cfg(feature = "blake3")]
mod by_hash;

pub(super) fn collect_ancestors<State, Cursor, Step>(
    store: &Store<State>,
    mut cursor: Option<Cursor>,
    limit: usize,
    mut step: Step,
) -> Vec<StoredEvent<serde_json::Value>>
where
    Step: FnMut(&Store<State>, Cursor) -> Option<(StoredEvent<serde_json::Value>, Option<Cursor>)>,
{
    let mut results = Vec::new();
    while results.len() < limit {
        let Some(current) = cursor.take() else {
            break;
        };
        let Some((stored, next)) = step(store, current) else {
            break;
        };
        results.push(stored);
        cursor = next;
    }
    results
}

pub(super) fn read_entry_and_event<State>(
    store: &Store<State>,
    event_id: u128,
) -> Option<(IndexEntry, StoredEvent<serde_json::Value>)> {
    let entry = store.index.get_by_id(event_id)?;
    let stored = store.reader.read_entry(&entry.disk_pos).ok()?;
    Some((entry, stored))
}

#[cfg(not(feature = "blake3"))]
pub(super) fn clock_cursor<State>(
    store: &Store<State>,
    event_id: u128,
) -> Option<std::vec::IntoIter<u128>> {
    let start_entry = store.index.get_by_id(event_id)?;
    let ids = store
        .index
        .stream(start_entry.coord.entity())
        .iter()
        .rev()
        .filter(|entry| entry.clock <= start_entry.clock)
        .map(|entry| entry.event_id)
        .collect::<Vec<_>>();
    Some(ids.into_iter())
}

pub(crate) fn walk_ancestors<State>(
    store: &Store<State>,
    event_id: u128,
    limit: usize,
) -> Vec<StoredEvent<serde_json::Value>> {
    #[cfg(feature = "blake3")]
    {
        by_hash::walk_ancestors_by_hash(store, event_id, limit)
    }

    #[cfg(not(feature = "blake3"))]
    {
        by_clock::walk_ancestors_by_clock(store, event_id, limit)
    }
}
