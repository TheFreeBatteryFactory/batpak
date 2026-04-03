use crate::coordinate::Coordinate;
use crate::event::{EventKind, HashChain};
use dashmap::DashMap;
use std::collections::{BTreeMap, HashSet};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;

/// StoreIndex: in-memory 2D index + auxiliaries. NOT persisted — rebuilt from segments on cold start.
/// [SPEC:src/store/index.rs]
/// [DEP:dashmap::DashMap] — see DEPENDENCY SURFACE for deadlock warnings
pub(crate) struct StoreIndex {
    /// Primary: entity -> ordered events. [DEP:dashmap::DashMap::get_mut] for insert.
    streams: DashMap<Arc<str>, BTreeMap<ClockKey, IndexEntry>>,
    /// Scope dimension: scope -> set of entities in that scope.
    scope_entities: DashMap<Arc<str>, HashSet<Arc<str>>>,
    /// Fact dimension: event kind -> ordered events of that kind.
    by_fact: DashMap<EventKind, BTreeMap<ClockKey, IndexEntry>>,
    /// Point lookup: event_id -> entry. O(1) get by ID.
    by_id: DashMap<u128, IndexEntry>,
    /// Chain head: entity -> latest IndexEntry. For prev_hash in writer step 2.
    latest: DashMap<Arc<str>, IndexEntry>,
    /// Monotonic counter. Foundation for cursors, checkpoints, exactly-once.
    global_sequence: AtomicU64,
    /// Total event count.
    len: AtomicUsize,
    /// Per-entity write locks. Writer step 1 acquires these.
    /// [SPEC:IMPLEMENTATION NOTES item 5 — DashMap guard lifetimes]
    pub(crate) entity_locks: DashMap<Arc<str>, Arc<parking_lot::Mutex<()>>>,
}

/// ClockKey: BTreeMap key. Ord: wall_ms-first, then clock, then uuid tiebreak.
/// wall_ms enables global causal ordering across entities (HLC layer 1).
/// [SPEC:IMPLEMENTATION NOTES item 1]

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClockKey {
    /// HLC wall clock milliseconds — global ordering across entities.
    pub wall_ms: u64,
    /// Per-entity monotonic sequence number used as the HLC logical counter.
    pub clock: u32,
    /// Event UUID tiebreaker for deterministic ordering within the same clock tick.
    pub uuid: u128,
}

/// IndexEntry: everything needed for index queries without disk reads.
#[derive(Clone, Debug)]
pub struct IndexEntry {
    /// Unique ID of the event.
    pub event_id: u128,
    /// Correlation ID linking related events in a causal chain.
    pub correlation_id: u128,
    /// ID of the event that caused this one; `None` for root-cause events.
    pub causation_id: Option<u128>,
    /// Entity and scope coordinates for this event.
    pub coord: Coordinate,
    /// Event kind (type discriminant).
    pub kind: EventKind,
    /// HLC wall clock milliseconds — for global causal ordering.
    pub wall_ms: u64,
    /// Per-entity monotonic sequence number.
    pub clock: u32,
    /// Blake3 hash chain linking this event to its predecessor.
    pub hash_chain: HashChain,
    /// Location of the event frame on disk.
    pub disk_pos: DiskPos,
    /// Globally monotonic sequence number assigned at commit time.
    pub global_sequence: u64,
}

/// DiskPos: where to find this event on disk.
#[derive(Clone, Debug)]
pub struct DiskPos {
    /// Numeric identifier of the segment file containing this event.
    pub segment_id: u64,
    /// Byte offset of the frame within the segment file.
    pub offset: u64,
    /// Total byte length of the encoded frame.
    pub length: u32,
}

impl Ord for ClockKey {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.wall_ms
            .cmp(&other.wall_ms)
            .then(self.clock.cmp(&other.clock))
            .then(self.uuid.cmp(&other.uuid))
    }
}

impl PartialOrd for ClockKey {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl IndexEntry {
    /// Returns `true` if this event is part of a causal chain (its correlation ID differs from its event ID).
    pub fn is_correlated(&self) -> bool {
        self.event_id != self.correlation_id
    }

    /// Returns `true` if this event was directly caused by the given event ID.
    pub fn is_caused_by(&self, event_id: u128) -> bool {
        self.causation_id == Some(event_id)
    }

    /// Returns `true` if this event has no causation ID (it is a root-cause event).
    pub fn is_root_cause(&self) -> bool {
        self.causation_id.is_none()
    }
}

impl StoreIndex {
    pub(crate) fn new() -> Self {
        Self {
            streams: DashMap::new(),
            scope_entities: DashMap::new(),
            by_fact: DashMap::new(),
            by_id: DashMap::new(),
            latest: DashMap::new(),
            global_sequence: AtomicU64::new(0),
            len: AtomicUsize::new(0),
            entity_locks: DashMap::new(),
        }
    }

    /// Called by writer step 9. Inserts into ALL indexes atomically.
    /// CONSTRAINT: caller MUST hold the entity lock before calling this.
    /// [SPEC:IMPLEMENTATION NOTES item 5]
    pub(crate) fn insert(&self, entry: IndexEntry) {
        let entity = entry.coord.entity_arc();
        let scope = entry.coord.scope_arc();
        let key = ClockKey {
            wall_ms: entry.wall_ms,
            clock: entry.clock,
            uuid: entry.event_id,
        };

        // Primary index: entity -> BTreeMap
        // [DEP:dashmap::DashMap::entry] — holds write lock, release fast
        self.streams
            .entry(Arc::clone(&entity))
            .or_default()
            .insert(key.clone(), entry.clone());

        // Scope index
        self.scope_entities
            .entry(scope)
            .or_default()
            .insert(Arc::clone(&entity));

        // Fact index
        self.by_fact
            .entry(entry.kind)
            .or_default()
            .insert(key, entry.clone());

        // Point lookup
        self.by_id.insert(entry.event_id, entry.clone());

        // Chain head
        self.latest.insert(entity, entry);

        // Counters
        self.global_sequence.fetch_add(1, Ordering::SeqCst);
        self.len.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn get_by_id(&self, event_id: u128) -> Option<IndexEntry> {
        self.by_id.get(&event_id).map(|r| r.value().clone())
    }

    pub(crate) fn get_latest(&self, entity: &str) -> Option<IndexEntry> {
        self.latest.get(entity).map(|r| r.value().clone())
    }

    pub(crate) fn stream(&self, entity: &str) -> Vec<IndexEntry> {
        self.streams
            .get(entity)
            .map(|r| r.value().values().cloned().collect())
            .unwrap_or_default()
    }

    pub(crate) fn query(&self, region: &crate::coordinate::Region) -> Vec<IndexEntry> {
        // Region query strategy:
        // 1. Determine candidate set based on most selective filter
        // 2. Apply remaining filters to narrow results
        // 3. Apply clock_range last (it's per-entity, cheap)
        use crate::coordinate::KindFilter;

        let mut candidates: Vec<IndexEntry> = if let Some(ref prefix) = region.entity_prefix {
            // Entity prefix → scan streams map for matching keys
            // [DEP:dashmap::DashMap::iter] — NOT a consistent snapshot, fine for queries
            self.streams
                .iter()
                .filter(|r| r.key().as_ref().starts_with(prefix.as_ref()))
                .flat_map(|r| r.value().values().cloned().collect::<Vec<_>>())
                .collect()
        } else if let Some(ref scope) = region.scope {
            // Scope → look up entities in scope, collect their streams
            if let Some(entities) = self.scope_entities.get(scope.as_ref()) {
                entities
                    .value()
                    .iter()
                    .flat_map(|entity| {
                        self.streams
                            .get(entity.as_ref())
                            .map(|r| r.value().values().cloned().collect::<Vec<_>>())
                            .unwrap_or_default()
                    })
                    .collect()
            } else {
                Vec::new()
            }
        } else if let Some(ref fact) = region.fact {
            // Fact filter without entity/scope → scan by_fact index
            match fact {
                KindFilter::Exact(k) => self
                    .by_fact
                    .get(k)
                    .map(|r| r.value().values().cloned().collect())
                    .unwrap_or_default(),
                KindFilter::Category(c) => {
                    let cat = *c;
                    self.by_fact
                        .iter()
                        .filter(|r| r.key().category() == cat)
                        .flat_map(|r| r.value().values().cloned().collect::<Vec<_>>())
                        .collect()
                }
                KindFilter::Any => {
                    // No filter at all — return everything (expensive, use sparingly)
                    self.streams
                        .iter()
                        .flat_map(|r| r.value().values().cloned().collect::<Vec<_>>())
                        .collect()
                }
            }
        } else {
            // Region::all() with no filters — return everything
            self.streams
                .iter()
                .flat_map(|r| r.value().values().cloned().collect::<Vec<_>>())
                .collect()
        };

        // Apply remaining filters that weren't used for the initial candidate set.

        // Scope filter (if entity_prefix was the primary selector)
        if region.entity_prefix.is_some() {
            if let Some(ref scope) = region.scope {
                candidates.retain(|e| e.coord.scope() == scope.as_ref());
            }
        }

        // Entity prefix filter: not needed here. When scope is the primary selector
        // and entity_prefix is Some, it's applied during initial candidate selection.

        // Fact filter (if not already applied)
        if region.entity_prefix.is_some() || region.scope.is_some() {
            if let Some(ref fact) = region.fact {
                candidates.retain(|e| match fact {
                    KindFilter::Exact(k) => e.kind == *k,
                    KindFilter::Category(c) => e.kind.category() == *c,
                    KindFilter::Any => true,
                });
            }
        }

        // Clock range filter (always per-entity clock, not global_sequence)
        if let Some((min, max)) = region.clock_range {
            candidates.retain(|e| e.clock >= min && e.clock <= max);
        }

        // Sort by global_sequence for consistent ordering
        candidates.sort_by_key(|e| e.global_sequence);
        candidates
    }

    pub(crate) fn global_sequence(&self) -> u64 {
        self.global_sequence.load(Ordering::SeqCst)
    }

    pub(crate) fn len(&self) -> usize {
        self.len.load(Ordering::Relaxed)
    }

    /// Clear all indexes for a full rebuild (e.g. after compaction).
    pub(crate) fn clear(&self) {
        self.streams.clear();
        self.scope_entities.clear();
        self.by_fact.clear();
        self.by_id.clear();
        self.latest.clear();
        self.global_sequence.store(0, Ordering::SeqCst);
        self.len.store(0, Ordering::Relaxed);
        // entity_locks intentionally NOT cleared — writer may hold references
    }
}
