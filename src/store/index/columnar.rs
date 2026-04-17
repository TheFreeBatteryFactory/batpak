//! Columnar (SoA / AoSoA) secondary query index.
//!
//! This module provides the columnar overlay indexes and the [`ScanIndex`]
//! fanout struct that maintains a mandatory AoS base view plus optional
//! SoA, SoAoS, and AoSoA64 overlays. All active views are populated on
//! every insert; queries route to the most efficient view for each access
//! pattern.
//!
//! ## Memory layout quick-reference
//!
//! | Variant   | Inner representation                                      |
//! |-----------|-----------------------------------------------------------|
//! | SoA       | Three `Vec`s sorted by `(kind, global_sequence)`          |
//! | AoSoA8    | `Vec<Tile<8>>`; each tile holds ≤ 8 events of one kind    |
//! | AoSoA16   | `Vec<Tile<16>>`; test-only tile-size harness               |
//! | AoSoA64   | `Vec<Tile<64>>`; cache-line aligned; scalar scan today     |
//!
//! ## Concurrency model
//!
//! `ColumnarIndex` wraps its mutable state in a single `parking_lot::RwLock`.
//! Multiple readers may query simultaneously; the writer thread takes an
//! exclusive write lock only during `insert`.  Because the writer already
//! serialises all appends before calling `StoreIndex::insert`, write
//! contention on this lock is effectively nil.
//!
//! ## Append ordering
//!
//! Events are always appended in ascending `global_sequence` order (the writer
//! thread assigns global_sequence under its own lock).  `insert` therefore
//! pushes to the back of the SoA vecs / open tile without any reordering.
//! `query_by_kind` performs a scalar linear pass for all layouts. AoSoA64 tiles
//! are sized to align with cache lines, but the current scan path does not use
//! SIMD intrinsics; that specialization is not yet implemented.

use crate::event::EventKind;
use crate::store::index::{projection_kind_matches, ClockKey, DiskPos, IndexEntry, RoutingSummary};
use dashmap::DashMap;
use parking_lot::RwLock;
use std::any::TypeId;
use std::collections::{BTreeMap, HashSet};
use std::sync::Arc;

type ProjectionCandidates = (u64, u64, Vec<(u64, DiskPos)>);

#[derive(Clone, Copy, Debug)]
enum EntryQuery<'a> {
    Kind(EventKind),
    Category(u8),
    Scope(&'a str),
}

// ---------------------------------------------------------------------------
// Tile — AoSoA building block
// ---------------------------------------------------------------------------

/// A tile that holds up to `N` events of the **same** kind.
///
/// `repr(C, align(64))` aligns the tile *struct header* (the fat-pointer fields)
/// to a 64-byte cache-line boundary. The inner `Vec`s allocate their backing
/// arrays on the heap separately, so kinds data is **not** cache-local to the
/// struct itself. The current scan is scalar and dereferences through the Vec
/// heap pointer on every access.
///
/// For a real SIMD specialization, `kinds` would need to be an inline array
/// (e.g. `[u16; N]`) so the kind values sit contiguously without a heap hop.
/// That restructuring is deferred until the specialization is actually implemented.
///
/// ### Why `Vec` instead of `[T; N]`?
///
/// Const-generic arrays of non-`Copy` types (e.g. `[Arc<IndexEntry>; N]`)
/// require `T: Default`, which `Arc<IndexEntry>` does not implement. Using
/// `Vec` with a reserved capacity of `N` avoids heap reallocation during a
/// tile's lifetime while keeping the code straightforward.
#[repr(C, align(64))]
pub(crate) struct Tile<const N: usize> {
    /// Event kinds stored in this tile; all entries have the same kind.
    pub kinds: Vec<EventKind>,
    /// Full index entries parallel to `kinds`.
    pub entries: Vec<Arc<IndexEntry>>,
    /// Number of valid elements currently stored in the tile.
    pub len: usize,
}

impl<const N: usize> Tile<N> {
    /// Create an empty tile pre-allocated to capacity `N`.
    pub(crate) fn new() -> Self {
        Self {
            kinds: Vec::with_capacity(N),
            entries: Vec::with_capacity(N),
            len: 0,
        }
    }

    /// Returns `true` when the tile has no room for another entry.
    #[inline]
    pub(crate) fn is_full(&self) -> bool {
        self.len >= N
    }

    /// Append an entry.  Panics (debug only) if the tile is already full.
    pub(crate) fn push(&mut self, kind: EventKind, entry: Arc<IndexEntry>) {
        debug_assert!(!self.is_full(), "Tile<{N}>::push called on a full tile");
        self.kinds.push(kind);
        self.entries.push(entry);
        self.len += 1;
    }
}

// ---------------------------------------------------------------------------
// SoAInner — the raw parallel-array state
// ---------------------------------------------------------------------------

/// Internal state for the flat SoA (Structure-of-Arrays) layout.
///
/// Events are stored in insertion order (== ascending `global_sequence`).
/// `query_by_kind` iterates linearly; because the `kinds` array is a compact
/// `Vec<u16>` (EventKind is a newtype over `u16`) the loop fits in L1 cache
/// for tens of thousands of events.
struct SoAInner {
    kinds: Vec<EventKind>,
    entries: Vec<Arc<IndexEntry>>,
    /// scope → set of entity strings that have emitted at least one event in
    /// that scope.  Mirrors the role of `StoreIndex::scope_entities`.
    scope_entities: std::collections::HashMap<Arc<str>, HashSet<Arc<str>>>,
}

impl SoAInner {
    fn new() -> Self {
        Self {
            kinds: Vec::new(),
            entries: Vec::new(),
            scope_entities: std::collections::HashMap::new(),
        }
    }

    fn from_entries(entries: &[Arc<IndexEntry>]) -> Self {
        let mut kinds = Vec::with_capacity(entries.len());
        let mut built_entries = Vec::with_capacity(entries.len());
        let mut scope_entities = std::collections::HashMap::<Arc<str>, HashSet<Arc<str>>>::new();

        for entry in entries {
            let scope = entry.coord.scope_arc();
            let entity = entry.coord.entity_arc();
            kinds.push(entry.kind);
            built_entries.push(Arc::clone(entry));
            scope_entities.entry(scope).or_default().insert(entity);
        }

        Self {
            kinds,
            entries: built_entries,
            scope_entities,
        }
    }

    /// Append one event.  O(1) amortised.
    fn push(&mut self, entry: &Arc<IndexEntry>) {
        let scope: Arc<str> = entry.coord.scope_arc();
        let entity: Arc<str> = entry.coord.entity_arc();
        self.kinds.push(entry.kind);
        self.entries.push(Arc::clone(entry));
        self.scope_entities.entry(scope).or_default().insert(entity);
    }

    fn query_entries(&self, mut matches: impl FnMut(EventKind) -> bool) -> Vec<Arc<IndexEntry>> {
        self.kinds
            .iter()
            .zip(self.entries.iter())
            .filter(|(kind, _)| matches(**kind))
            .map(|(_, e)| Arc::clone(e))
            .collect()
    }

    /// Return all entries whose `kind == target`.  Linear scan; cache-friendly
    /// because `kinds` is a packed `Vec<EventKind>` (2 bytes per element).
    fn query_by_kind(&self, target: EventKind) -> Vec<Arc<IndexEntry>> {
        self.query_entries(|kind| kind == target)
    }

    /// Return all entries whose kind falls in `category` (upper 4 bits).
    fn query_by_category(&self, category: u8) -> Vec<Arc<IndexEntry>> {
        self.query_entries(|kind| kind.category() == category)
    }

    /// Return all entries belonging to entities registered under `scope`.
    fn query_by_scope(&self, scope: &str) -> Vec<Arc<IndexEntry>> {
        let Some(entities) = self.scope_entities.get(scope) else {
            return Vec::new();
        };
        self.entries
            .iter()
            .filter(|e| entities.contains(e.coord.entity_arc().as_ref()))
            .map(Arc::clone)
            .collect()
    }

    fn candidates(&self, spec: &EntryQuery<'_>) -> Vec<Arc<IndexEntry>> {
        match spec {
            EntryQuery::Kind(k) => self.query_by_kind(*k),
            EntryQuery::Category(c) => self.query_by_category(*c),
            EntryQuery::Scope(s) => self.query_by_scope(s),
        }
    }

    fn clear(&mut self) {
        self.kinds.clear();
        self.entries.clear();
        self.scope_entities.clear();
    }
}

// ---------------------------------------------------------------------------
// AoSoAInner — tiled parallel-array state (generic over tile width N)
// ---------------------------------------------------------------------------

/// Internal state for tiled AoSoA layouts.
///
/// Events are bucketed into tiles by kind: every tile contains entries of a
/// single `EventKind` (matching `kinds[0]` for any non-empty tile). Each kind
/// has at most one open tile at a time; `open_tiles` maps a kind to the index
/// of its current open tile. When a tile fills, it is evicted from `open_tiles`
/// and a new tile is started on the next event of that kind.
///
/// This strategy keeps tiles full regardless of insertion order, so interleaved
/// multi-kind workloads produce the same tile density as sorted runs.
///
/// The outer `Vec` of `Tile`s is unsorted; `query_by_kind` iterates all tiles
/// and collects matching entries. Tiles are cache-line aligned, but the current
/// scan is scalar. The tile structure is the correct layout for a future SIMD
/// specialization; see the AoSoA64 variant.
struct AoSoAInner<const N: usize> {
    tiles: Vec<Tile<N>>,
    /// kind → index of the currently open (not yet full) tile for that kind.
    open_tiles: std::collections::HashMap<EventKind, usize>,
    /// scope → entity set, same role as in SoAInner.
    scope_entities: std::collections::HashMap<Arc<str>, HashSet<Arc<str>>>,
}

impl<const N: usize> AoSoAInner<N> {
    fn new() -> Self {
        Self {
            tiles: Vec::new(),
            open_tiles: std::collections::HashMap::new(),
            scope_entities: std::collections::HashMap::new(),
        }
    }

    fn from_entries(entries: &[Arc<IndexEntry>]) -> Self {
        let mut built = Self::new();
        for entry in entries {
            built.push(entry);
        }
        built
    }

    /// Append one event into the appropriate tile.
    ///
    /// Each kind has at most one open tile. If the open tile for this kind is
    /// full (or none exists), a new tile is allocated and registered as open.
    fn push(&mut self, entry: &Arc<IndexEntry>) {
        let scope: Arc<str> = entry.coord.scope_arc();
        let entity: Arc<str> = entry.coord.entity_arc();
        let kind = entry.kind;

        match self.open_tiles.get(&kind).copied() {
            Some(idx) => {
                self.tiles[idx].push(kind, Arc::clone(entry));
                if self.tiles[idx].is_full() {
                    self.open_tiles.remove(&kind);
                }
            }
            None => {
                let new_idx = self.tiles.len();
                let mut tile = Tile::new();
                tile.push(kind, Arc::clone(entry));
                let is_full = tile.is_full();
                self.tiles.push(tile);
                if !is_full {
                    self.open_tiles.insert(kind, new_idx);
                }
            }
        }

        self.scope_entities.entry(scope).or_default().insert(entity);
    }

    fn query_entries(&self, mut matches: impl FnMut(EventKind) -> bool) -> Vec<Arc<IndexEntry>> {
        let mut out = Vec::new();
        for tile in &self.tiles {
            // All elements in a tile share the same kind; skip non-matching tiles fast.
            if tile
                .kinds
                .first()
                .copied()
                .is_none_or(|kind| !matches(kind))
            {
                continue;
            }
            for e in tile.entries.iter().take(tile.len) {
                out.push(Arc::clone(e));
            }
        }
        out
    }

    /// Iterate every tile and collect entries whose kind matches `target`.
    fn query_by_kind(&self, target: EventKind) -> Vec<Arc<IndexEntry>> {
        self.query_entries(|kind| kind == target)
    }

    /// Return all entries whose kind falls in `category` (upper 4 bits).
    /// Skips entire tiles whose kind doesn't match the category.
    fn query_by_category(&self, category: u8) -> Vec<Arc<IndexEntry>> {
        self.query_entries(|kind| kind.category() == category)
    }

    /// Collect entries belonging to entities in `scope`.
    fn query_by_scope(&self, scope: &str) -> Vec<Arc<IndexEntry>> {
        let Some(entities) = self.scope_entities.get(scope) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for tile in &self.tiles {
            for e in tile.entries.iter().take(tile.len) {
                if entities.contains(e.coord.entity_arc().as_ref()) {
                    out.push(Arc::clone(e));
                }
            }
        }
        out
    }

    fn candidates(&self, spec: &EntryQuery<'_>) -> Vec<Arc<IndexEntry>> {
        match spec {
            EntryQuery::Kind(k) => self.query_by_kind(*k),
            EntryQuery::Category(c) => self.query_by_category(*c),
            EntryQuery::Scope(s) => self.query_by_scope(s),
        }
    }

    /// Execute `f` on the tile at position `idx`.
    ///
    /// Returns `None` if `idx` is out of range.
    pub(crate) fn with_tile<R>(&self, idx: usize, f: impl FnOnce(&Tile<N>) -> R) -> Option<R> {
        self.tiles.get(idx).map(f)
    }

    fn clear(&mut self) {
        self.tiles.clear();
        self.open_tiles.clear();
        self.scope_entities.clear();
    }
}

// ---------------------------------------------------------------------------
// ColumnarVariant — erases the const-generic parameter at the enum level
// ── SoAoS: hybrid AoS-outer, SoA-inner ──────────────────────────────────────

/// One entity's events stored as parallel arrays (SoA within an entity group).
#[derive(Clone, Debug)]
pub(crate) struct CachedProjectionSlot {
    pub(crate) bytes: Vec<u8>,
    pub(crate) watermark: u64,
    pub(crate) generation: u64,
    pub(crate) cached_at_us: i64,
}

struct EntityGroup {
    kinds: Vec<EventKind>,
    entries: Vec<Arc<IndexEntry>>,
    generation: u64,
    cached_projections: std::collections::HashMap<TypeId, CachedProjectionSlot>,
}

/// Hybrid layout: entities looked up by HashMap (AoS outer), events within each
/// entity stored as parallel arrays (SoA inner). Matches the ECS archetype pattern.
struct SoAoSInner {
    groups: std::collections::HashMap<Arc<str>, EntityGroup>,
    scope_entities: std::collections::HashMap<Arc<str>, std::collections::HashSet<Arc<str>>>,
}

impl SoAoSInner {
    fn new() -> Self {
        Self {
            groups: std::collections::HashMap::new(),
            scope_entities: std::collections::HashMap::new(),
        }
    }

    fn from_restore_base(entries_by_entity: &[Arc<IndexEntry>], routing: &RoutingSummary) -> Self {
        let mut groups = std::collections::HashMap::with_capacity(routing.entity_runs.len());
        let mut scope_entities =
            std::collections::HashMap::<Arc<str>, std::collections::HashSet<Arc<str>>>::new();

        for run in &routing.entity_runs {
            let start = usize::try_from(run.start)
                .expect("invariant: entity run index fits usize on any supported target");
            let end = start
                + usize::try_from(run.len)
                    .expect("invariant: entity run length fits usize on any supported target");
            let slice = &entries_by_entity[start..end];
            if slice.is_empty() {
                continue;
            }
            let entity = slice[0].coord.entity_arc();
            let mut group = EntityGroup {
                kinds: Vec::with_capacity(slice.len()),
                entries: Vec::with_capacity(slice.len()),
                generation: slice.len() as u64,
                cached_projections: std::collections::HashMap::new(),
            };
            for entry in slice {
                group.kinds.push(entry.kind);
                group.entries.push(Arc::clone(entry));
                scope_entities
                    .entry(entry.coord.scope_arc())
                    .or_default()
                    .insert(Arc::clone(&entity));
            }
            groups.insert(entity, group);
        }

        Self {
            groups,
            scope_entities,
        }
    }

    fn push(&mut self, entry: &Arc<IndexEntry>) {
        let entity = entry.coord.entity_arc();
        let scope = entry.coord.scope_arc();
        let group = self
            .groups
            .entry(Arc::clone(&entity))
            .or_insert_with(|| EntityGroup {
                kinds: Vec::new(),
                entries: Vec::new(),
                generation: 0,
                cached_projections: std::collections::HashMap::new(),
            });
        group.kinds.push(entry.kind);
        group.entries.push(Arc::clone(entry));
        group.generation = group.generation.saturating_add(1);
        self.scope_entities.entry(scope).or_default().insert(entity);
    }

    fn query_entries(&self, mut matches: impl FnMut(EventKind) -> bool) -> Vec<Arc<IndexEntry>> {
        let mut out = Vec::new();
        for group in self.groups.values() {
            for (i, &kind) in group.kinds.iter().enumerate() {
                if matches(kind) {
                    out.push(Arc::clone(&group.entries[i]));
                }
            }
        }
        out
    }

    fn query_by_kind(&self, target: EventKind) -> Vec<Arc<IndexEntry>> {
        self.query_entries(|kind| kind == target)
    }

    fn query_by_category(&self, category: u8) -> Vec<Arc<IndexEntry>> {
        self.query_entries(|kind| kind.category() == category)
    }

    fn query_by_scope(&self, scope: &str) -> Vec<Arc<IndexEntry>> {
        let mut out = Vec::new();
        if let Some(entities) = self.scope_entities.get(scope) {
            for entity in entities {
                if let Some(group) = self.groups.get(entity.as_ref()) {
                    out.extend(group.entries.iter().map(Arc::clone));
                }
            }
        }
        out
    }

    fn candidates(&self, spec: &EntryQuery<'_>) -> Vec<Arc<IndexEntry>> {
        match spec {
            EntryQuery::Kind(k) => self.query_by_kind(*k),
            EntryQuery::Category(c) => self.query_by_category(*c),
            EntryQuery::Scope(s) => self.query_by_scope(s),
        }
    }

    fn entity_generation(&self, entity: &str) -> Option<u64> {
        self.groups.get(entity).map(|group| group.generation)
    }

    fn projection_candidates(
        &self,
        entity: &str,
        relevant_kinds: &[EventKind],
    ) -> Option<ProjectionCandidates> {
        let group = self.groups.get(entity)?;
        let mut candidates = Vec::new();
        let mut watermark = None;

        for (&kind, entry) in group.kinds.iter().zip(group.entries.iter()) {
            if !projection_kind_matches(relevant_kinds, kind) {
                continue;
            }
            let sequence = entry.global_sequence;
            watermark = Some(sequence);
            candidates.push((sequence, entry.disk_pos));
        }

        Some((watermark?, group.generation, candidates))
    }

    fn cached_projection(&self, entity: &str, type_id: TypeId) -> Option<CachedProjectionSlot> {
        self.groups
            .get(entity)
            .and_then(|group| group.cached_projections.get(&type_id).cloned())
    }

    fn store_cached_projection(
        &mut self,
        entity: &str,
        type_id: TypeId,
        bytes: Vec<u8>,
        watermark: u64,
        cached_at_us: i64,
    ) -> bool {
        let Some(group) = self.groups.get_mut(entity) else {
            return false;
        };
        group.cached_projections.insert(
            type_id,
            CachedProjectionSlot {
                bytes,
                watermark,
                generation: group.generation,
                cached_at_us,
            },
        );
        true
    }

    fn clear(&mut self) {
        self.groups.clear();
        self.scope_entities.clear();
    }
}

// ---------------------------------------------------------------------------

/// Concrete storage variant held inside a [`ColumnarIndex`].
///
/// Each arm holds the mutable inner state behind a `RwLock` so that
/// concurrent readers never block each other.
enum ColumnarVariant {
    /// Flat parallel arrays; best for sequential scans.
    SoA(RwLock<SoAInner>),
    #[cfg(test)]
    /// 8-element tiles; test-only tile-size regression harness, not a production path.
    AoSoA8(RwLock<AoSoAInner<8>>),
    #[cfg(test)]
    /// 16-element tiles; test-only tile-size regression harness, not a production path.
    AoSoA16(RwLock<AoSoAInner<16>>),
    /// 64-element tiles; cache-line aligned, scalar scan today.
    ///
    /// **Routing decision (2026-04-17):** AoSoA64 is safe on all corpus shapes
    /// after the kind-keyed fill fix. Benchmarked scalar path shows ~5–9% win
    /// over SoA on sorted by_kind; interleaved is now at parity. Does not yet
    /// clear the 15% routing threshold. SIMD executor (by_kind, by_category) is
    /// the next lever — implement only after benchmarking confirms the tile
    /// structure earns the route on target hardware.
    AoSoA64(RwLock<AoSoAInner<64>>),
    /// Hybrid AoS-outer (entity groups), SoA-inner (parallel arrays per entity).
    SoAoS(RwLock<SoAoSInner>),
}

// ---------------------------------------------------------------------------
// ColumnarIndex — public API
// ---------------------------------------------------------------------------

/// Cache-friendly secondary query index that supplements the `by_fact` and
/// `scope_entities` `DashMap`s with an optional columnar overlay.
///
/// ## Thread safety
///
/// All methods take `&self`; internal state is protected by a
/// `parking_lot::RwLock`.  Writers hold an exclusive lock for the duration of
/// [`insert`]; readers share a read lock.  Because the writer thread
/// serialises all appends, write contention is negligible.
///
/// [`insert`]: ColumnarIndex::insert
pub(crate) struct ColumnarIndex {
    inner: ColumnarVariant,
}

impl ColumnarIndex {
    /// Create a new flat SoA index.
    pub(crate) fn new_soa() -> Self {
        Self {
            inner: ColumnarVariant::SoA(RwLock::new(SoAInner::new())),
        }
    }

    #[cfg(test)]
    /// Create a new AoSoA index with 8-element tiles.
    pub(crate) fn new_aosoa8() -> Self {
        Self {
            inner: ColumnarVariant::AoSoA8(RwLock::new(AoSoAInner::<8>::new())),
        }
    }

    #[cfg(test)]
    /// Create a new AoSoA index with 16-element tiles.
    pub(crate) fn new_aosoa16() -> Self {
        Self {
            inner: ColumnarVariant::AoSoA16(RwLock::new(AoSoAInner::<16>::new())),
        }
    }

    /// Create a new AoSoA index with 64-element tiles.
    pub(crate) fn new_aosoa64() -> Self {
        Self {
            inner: ColumnarVariant::AoSoA64(RwLock::new(AoSoAInner::<64>::new())),
        }
    }

    /// Create a new SoAoS (hybrid AoS-outer, SoA-inner) index.
    pub(crate) fn new_soaos() -> Self {
        Self {
            inner: ColumnarVariant::SoAoS(RwLock::new(SoAoSInner::new())),
        }
    }

    /// Append `entry` to the index.
    ///
    /// Events must be inserted in ascending `global_sequence` order (which is
    /// guaranteed by the single-writer architecture).  The operation is O(1)
    /// amortised for SoA and O(1) amortised for AoSoA (tile append or new tile).
    pub(crate) fn insert(&self, entry: &Arc<IndexEntry>) {
        match &self.inner {
            ColumnarVariant::SoA(lock) => lock.write().push(entry),
            #[cfg(test)]
            ColumnarVariant::AoSoA8(lock) => lock.write().push(entry),
            #[cfg(test)]
            ColumnarVariant::AoSoA16(lock) => lock.write().push(entry),
            ColumnarVariant::AoSoA64(lock) => lock.write().push(entry),
            ColumnarVariant::SoAoS(lock) => lock.write().push(entry),
        }
    }

    pub(crate) fn rebuild_from_restore_base(
        &self,
        entries_by_sequence: &[Arc<IndexEntry>],
        entries_by_entity: &[Arc<IndexEntry>],
        routing: &RoutingSummary,
    ) {
        match &self.inner {
            ColumnarVariant::SoA(lock) => {
                *lock.write() = SoAInner::from_entries(entries_by_sequence)
            }
            #[cfg(test)]
            ColumnarVariant::AoSoA8(lock) => {
                *lock.write() = AoSoAInner::<8>::from_entries(entries_by_sequence)
            }
            #[cfg(test)]
            ColumnarVariant::AoSoA16(lock) => {
                *lock.write() = AoSoAInner::<16>::from_entries(entries_by_sequence)
            }
            ColumnarVariant::AoSoA64(lock) => {
                *lock.write() = AoSoAInner::<64>::from_entries(entries_by_sequence)
            }
            ColumnarVariant::SoAoS(lock) => {
                *lock.write() = SoAoSInner::from_restore_base(entries_by_entity, routing)
            }
        }
    }

    /// Return all entries matching `query`, sorted by `global_sequence`
    /// (ascending).
    ///
    /// Dispatch chooses the layout; each layout's `candidates()` applies its own
    /// native optimization (tile-skip for AoSoA, group lookup for SoAoS). The
    /// sort is the only shared semantic: result order is stable across layouts.
    fn query_sorted(&self, query: EntryQuery<'_>) -> Vec<Arc<IndexEntry>> {
        let mut results = match &self.inner {
            ColumnarVariant::SoA(lock) => lock.read().candidates(&query),
            ColumnarVariant::AoSoA64(lock) => lock.read().candidates(&query),
            ColumnarVariant::SoAoS(lock) => lock.read().candidates(&query),
            #[cfg(test)]
            ColumnarVariant::AoSoA8(lock) => lock.read().candidates(&query),
            #[cfg(test)]
            ColumnarVariant::AoSoA16(lock) => lock.read().candidates(&query),
        };
        results.sort_by_key(|e| e.global_sequence);
        results
    }

    /// Return all entries whose `kind` exactly matches `target`, sorted by
    /// `global_sequence` (ascending).
    pub(crate) fn query_by_kind(&self, target: EventKind) -> Vec<Arc<IndexEntry>> {
        self.query_sorted(EntryQuery::Kind(target))
    }

    /// Return all entries whose kind falls in `category` (upper 4 bits),
    /// sorted by `global_sequence` (ascending).
    pub(crate) fn query_by_category(&self, category: u8) -> Vec<Arc<IndexEntry>> {
        self.query_sorted(EntryQuery::Category(category))
    }

    /// Return all entries whose coordinate scope matches `scope`, sorted by
    /// `global_sequence` (ascending).
    pub(crate) fn query_by_scope(&self, scope: &str) -> Vec<Arc<IndexEntry>> {
        self.query_sorted(EntryQuery::Scope(scope))
    }

    /// Invoke `f` with an immutable reference to the `Tile<8>` at `idx`.
    ///
    /// This callback pattern avoids exposing interior mutability outside the
    /// module and prevents callers from holding a `RwLockReadGuard` longer
    /// than necessary.
    ///
    /// # Panics
    /// Panics if `self` is not an `AoSoA8` variant, or if `idx` is out of range.
    /// Caller contract violation — not recoverable.
    /// Invoke `f` with an immutable reference to the `Tile<8>` at `idx`.
    /// Returns `None` if `self` is not an `AoSoA8` variant.
    #[cfg(test)]
    fn with_tile8<R>(&self, idx: usize, f: impl FnOnce(&Tile<8>) -> R) -> Option<R> {
        match &self.inner {
            ColumnarVariant::AoSoA8(lock) => lock.read().with_tile(idx, f),
            ColumnarVariant::SoA(_) | ColumnarVariant::AoSoA64(_) | ColumnarVariant::SoAoS(_) => {
                None
            }
            #[cfg(test)]
            ColumnarVariant::AoSoA16(_) => None,
        }
    }

    /// Invoke `f` with an immutable reference to the `Tile<16>` at `idx`.
    /// Returns `None` if `self` is not an `AoSoA16` variant or idx is out of range.
    #[cfg(test)]
    fn with_tile16<R>(&self, idx: usize, f: impl FnOnce(&Tile<16>) -> R) -> Option<R> {
        match &self.inner {
            ColumnarVariant::AoSoA16(lock) => lock.read().with_tile(idx, f),
            ColumnarVariant::SoA(_) | ColumnarVariant::AoSoA64(_) | ColumnarVariant::SoAoS(_) => {
                None
            }
            #[cfg(test)]
            ColumnarVariant::AoSoA8(_) => None,
        }
    }

    /// Invoke `f` with an immutable reference to the `Tile<64>` at `idx`.
    /// Returns `None` if `self` is not an `AoSoA64` variant or idx is out of range.
    fn with_tile64<R>(&self, idx: usize, f: impl FnOnce(&Tile<64>) -> R) -> Option<R> {
        match &self.inner {
            ColumnarVariant::AoSoA64(lock) => lock.read().with_tile(idx, f),
            ColumnarVariant::SoA(_) | ColumnarVariant::SoAoS(_) => None,
            #[cfg(test)]
            ColumnarVariant::AoSoA8(_) | ColumnarVariant::AoSoA16(_) => None,
        }
    }

    /// Discard all entries.  Called during index rebuild (compaction / cold start).
    pub(crate) fn clear(&self) {
        match &self.inner {
            ColumnarVariant::SoA(lock) => lock.write().clear(),
            #[cfg(test)]
            ColumnarVariant::AoSoA8(lock) => lock.write().clear(),
            #[cfg(test)]
            ColumnarVariant::AoSoA16(lock) => lock.write().clear(),
            ColumnarVariant::AoSoA64(lock) => lock.write().clear(),
            ColumnarVariant::SoAoS(lock) => lock.write().clear(),
        }
    }

    /// Return the number of tiles for the production tiled overlay, or 0 for
    /// non-tiled layouts.
    pub(crate) fn tile_count(&self) -> usize {
        if self.with_tile64(0, |_| ()).is_some() {
            if let ColumnarVariant::AoSoA64(lock) = &self.inner {
                return lock.read().tiles.len();
            }
        }
        0
    }

    pub(crate) fn entity_generation(&self, entity: &str) -> Option<u64> {
        match &self.inner {
            ColumnarVariant::SoAoS(lock) => lock.read().entity_generation(entity),
            ColumnarVariant::SoA(_) | ColumnarVariant::AoSoA64(_) => None,
            #[cfg(test)]
            ColumnarVariant::AoSoA8(_) | ColumnarVariant::AoSoA16(_) => None,
        }
    }

    pub(crate) fn cached_projection(
        &self,
        entity: &str,
        type_id: TypeId,
    ) -> Option<CachedProjectionSlot> {
        match &self.inner {
            ColumnarVariant::SoAoS(lock) => lock.read().cached_projection(entity, type_id),
            ColumnarVariant::SoA(_) | ColumnarVariant::AoSoA64(_) => None,
            #[cfg(test)]
            ColumnarVariant::AoSoA8(_) | ColumnarVariant::AoSoA16(_) => None,
        }
    }

    pub(crate) fn store_cached_projection(
        &self,
        entity: &str,
        type_id: TypeId,
        bytes: Vec<u8>,
        watermark: u64,
        cached_at_us: i64,
    ) -> bool {
        match &self.inner {
            ColumnarVariant::SoAoS(lock) => lock.write().store_cached_projection(
                entity,
                type_id,
                bytes,
                watermark,
                cached_at_us,
            ),
            ColumnarVariant::SoA(_) | ColumnarVariant::AoSoA64(_) => false,
            #[cfg(test)]
            ColumnarVariant::AoSoA8(_) | ColumnarVariant::AoSoA16(_) => false,
        }
    }

    pub(crate) fn projection_candidates(
        &self,
        entity: &str,
        relevant_kinds: &[EventKind],
    ) -> Option<ProjectionCandidates> {
        match &self.inner {
            ColumnarVariant::SoAoS(lock) => {
                lock.read().projection_candidates(entity, relevant_kinds)
            }
            ColumnarVariant::SoA(_) | ColumnarVariant::AoSoA64(_) => None,
            #[cfg(test)]
            ColumnarVariant::AoSoA8(_) | ColumnarVariant::AoSoA16(_) => None,
        }
    }
}

// ---------------------------------------------------------------------------
// ScanIndex — top-level dispatcher
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ScanRoute {
    BaseAoS,
    SoA,
    SoAoS,
    AoSoA64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ProjectionSupport {
    entity_generation_fast_path: bool,
    cached_projection: bool,
    projection_candidates: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ScanCapabilities {
    by_kind: ScanRoute,
    by_scope: ScanRoute,
    by_category: ScanRoute,
    projection: ProjectionSupport,
    topology_name: &'static str,
    tile_count: usize,
}

/// Base AoS indexes plus optional multi-view overlays.
pub(crate) struct ScanIndex {
    /// Event-kind → ordered event entries.
    by_fact: DashMap<EventKind, BTreeMap<ClockKey, Arc<IndexEntry>>>,
    /// Scope string → set of entity strings active in that scope.
    scope_entities: DashMap<Arc<str>, HashSet<Arc<str>>>,
    /// Broad-scan overlay.
    soa: Option<ColumnarIndex>,
    /// Entity-group overlay.
    entity_groups: Option<ColumnarIndex>,
    /// Tiled replay/scanning overlay.
    tiles64: Option<ColumnarIndex>,
}

impl ScanIndex {
    /// Construct base AoS maps plus the configured optional overlays.
    pub(crate) fn for_config(config: &crate::store::IndexConfig) -> Self {
        let soa = config.topology.soa_enabled();
        let entity_groups = config.topology.entity_groups_enabled();
        let tiles64 = config.topology.tiles64_enabled();

        Self {
            by_fact: DashMap::new(),
            scope_entities: DashMap::new(),
            soa: soa.then(ColumnarIndex::new_soa),
            entity_groups: entity_groups.then(ColumnarIndex::new_soaos),
            tiles64: tiles64.then(ColumnarIndex::new_aosoa64),
        }
    }

    fn insert_base(&self, entry: &Arc<IndexEntry>) {
        let key = ClockKey {
            wall_ms: entry.wall_ms,
            clock: entry.clock,
            uuid: entry.event_id,
        };
        self.by_fact
            .entry(entry.kind)
            .or_default()
            .insert(key, Arc::clone(entry));
        self.scope_entities
            .entry(entry.coord.scope_arc())
            .or_default()
            .insert(entry.coord.entity_arc());
    }

    fn query_base_by_kind(&self, kind: EventKind) -> Vec<Arc<IndexEntry>> {
        let mut results: Vec<Arc<IndexEntry>> = self
            .by_fact
            .get(&kind)
            .map(|r| r.value().values().map(Arc::clone).collect())
            .unwrap_or_default();
        results.sort_by_key(|e| e.global_sequence);
        results
    }

    fn query_base_by_category(&self, category: u8) -> Vec<Arc<IndexEntry>> {
        let mut results = Vec::new();
        for entries in self
            .by_fact
            .iter()
            .filter(|r| r.key().category() == category)
        {
            results.extend(entries.value().values().map(Arc::clone));
        }
        results.sort_by_key(|e| e.global_sequence);
        results
    }

    fn capabilities(&self) -> ScanCapabilities {
        let (has_soa, has_entity_groups, has_tiles64) = (
            self.soa.is_some(),
            self.entity_groups.is_some(),
            self.tiles64.is_some(),
        );

        let topology_name = match (has_soa, has_entity_groups, has_tiles64) {
            (false, false, false) => "aos",
            (true, false, false) => "scan",
            (false, true, false) => "entity-local",
            (false, false, true) => "tiled",
            (true, true, true) => "all",
            _ => "hybrid",
        };

        ScanCapabilities {
            by_kind: if has_soa {
                ScanRoute::SoA
            } else if has_tiles64 {
                ScanRoute::AoSoA64
            } else if has_entity_groups {
                ScanRoute::SoAoS
            } else {
                ScanRoute::BaseAoS
            },
            by_scope: if has_entity_groups {
                ScanRoute::SoAoS
            } else if has_soa {
                ScanRoute::SoA
            } else if has_tiles64 {
                ScanRoute::AoSoA64
            } else {
                ScanRoute::BaseAoS
            },
            by_category: if has_soa {
                ScanRoute::SoA
            } else if has_tiles64 {
                ScanRoute::AoSoA64
            } else if has_entity_groups {
                ScanRoute::SoAoS
            } else {
                ScanRoute::BaseAoS
            },
            projection: ProjectionSupport {
                entity_generation_fast_path: has_entity_groups,
                cached_projection: has_entity_groups,
                projection_candidates: has_entity_groups,
            },
            topology_name,
            tile_count: self.tiles64.as_ref().map_or(0, ColumnarIndex::tile_count),
        }
    }

    fn query_route(&self, route: ScanRoute, query: EntryQuery<'_>) -> Vec<Arc<IndexEntry>> {
        match (route, query) {
            (ScanRoute::BaseAoS, EntryQuery::Kind(kind)) => self.query_base_by_kind(kind),
            (ScanRoute::BaseAoS, EntryQuery::Category(category)) => {
                self.query_base_by_category(category)
            }
            // Base AoS keeps the authoritative scope -> entity set, but not a
            // direct scope -> entries index. Callers pair this route with
            // `scope_entity_set` when they need the full entry list.
            (ScanRoute::BaseAoS, EntryQuery::Scope(_)) => Vec::new(),
            (ScanRoute::SoA, EntryQuery::Kind(kind)) => self
                .soa
                .as_ref()
                .expect("ScanCapabilities routed queries through missing SoA overlay")
                .query_by_kind(kind),
            (ScanRoute::SoA, EntryQuery::Category(category)) => self
                .soa
                .as_ref()
                .expect("ScanCapabilities routed queries through missing SoA overlay")
                .query_by_category(category),
            (ScanRoute::SoA, EntryQuery::Scope(scope)) => self
                .soa
                .as_ref()
                .expect("ScanCapabilities routed queries through missing SoA overlay")
                .query_by_scope(scope),
            (ScanRoute::SoAoS, EntryQuery::Kind(kind)) => self
                .entity_groups
                .as_ref()
                .expect("ScanCapabilities routed queries through missing SoAoS overlay")
                .query_by_kind(kind),
            (ScanRoute::SoAoS, EntryQuery::Category(category)) => self
                .entity_groups
                .as_ref()
                .expect("ScanCapabilities routed queries through missing SoAoS overlay")
                .query_by_category(category),
            (ScanRoute::SoAoS, EntryQuery::Scope(scope)) => self
                .entity_groups
                .as_ref()
                .expect("ScanCapabilities routed queries through missing SoAoS overlay")
                .query_by_scope(scope),
            (ScanRoute::AoSoA64, EntryQuery::Kind(kind)) => self
                .tiles64
                .as_ref()
                .expect("ScanCapabilities routed queries through missing AoSoA64 overlay")
                .query_by_kind(kind),
            (ScanRoute::AoSoA64, EntryQuery::Category(category)) => self
                .tiles64
                .as_ref()
                .expect("ScanCapabilities routed queries through missing AoSoA64 overlay")
                .query_by_category(category),
            (ScanRoute::AoSoA64, EntryQuery::Scope(scope)) => self
                .tiles64
                .as_ref()
                .expect("ScanCapabilities routed queries through missing AoSoA64 overlay")
                .query_by_scope(scope),
        }
    }

    pub(crate) fn topology_name(&self) -> &'static str {
        self.capabilities().topology_name
    }

    pub(crate) fn tile_count(&self) -> usize {
        self.capabilities().tile_count
    }

    /// Insert an entry into whichever secondary index is active.
    ///
    /// For `Maps`, this updates both `by_fact` and `scope_entities` using the
    /// same `ClockKey` ordering used by `StoreIndex::insert`.
    ///
    /// For `Columnar`, this delegates to [`ColumnarIndex::insert`].
    pub(crate) fn insert(&self, entry: &Arc<IndexEntry>) {
        self.insert_base(entry);
        if let Some(idx) = &self.soa {
            idx.insert(entry);
        }
        if let Some(idx) = &self.entity_groups {
            idx.insert(entry);
        }
        if let Some(idx) = &self.tiles64 {
            idx.insert(entry);
        }
    }

    pub(crate) fn rebuild_from_restore_base(
        &self,
        entries_by_sequence: &[Arc<IndexEntry>],
        entries_by_entity: &[Arc<IndexEntry>],
        routing: &RoutingSummary,
    ) {
        self.by_fact.clear();
        self.scope_entities.clear();

        let mut by_fact =
            std::collections::HashMap::<EventKind, BTreeMap<ClockKey, Arc<IndexEntry>>>::new();
        let mut scope_entities = std::collections::HashMap::<Arc<str>, HashSet<Arc<str>>>::new();

        for entry in entries_by_sequence {
            let key = ClockKey {
                wall_ms: entry.wall_ms,
                clock: entry.clock,
                uuid: entry.event_id,
            };
            by_fact
                .entry(entry.kind)
                .or_default()
                .insert(key, Arc::clone(entry));
            scope_entities
                .entry(entry.coord.scope_arc())
                .or_default()
                .insert(entry.coord.entity_arc());
        }

        for (kind, map) in by_fact {
            self.by_fact.insert(kind, map);
        }
        for (scope, entities) in scope_entities {
            self.scope_entities.insert(scope, entities);
        }

        if let Some(idx) = &self.soa {
            idx.rebuild_from_restore_base(entries_by_sequence, entries_by_entity, routing);
        }
        if let Some(idx) = &self.entity_groups {
            idx.rebuild_from_restore_base(entries_by_sequence, entries_by_entity, routing);
        }
        if let Some(idx) = &self.tiles64 {
            idx.rebuild_from_restore_base(entries_by_sequence, entries_by_entity, routing);
        }
    }

    /// Return all entries matching `kind`, sorted by `global_sequence`.
    ///
    /// For `Maps`, this clones values out of the `BTreeMap` (ordered by
    /// `ClockKey`, which is equivalent to `global_sequence` order for events
    /// that belong to the same entity stream; a final sort ensures correctness
    /// across entities).
    ///
    /// For `Columnar`, delegates to [`ColumnarIndex::query_by_kind`].
    pub(crate) fn query_by_kind(&self, kind: EventKind) -> Vec<Arc<IndexEntry>> {
        self.query_route(self.capabilities().by_kind, EntryQuery::Kind(kind))
    }

    /// Return all entries whose coordinate scope matches `scope`, sorted by
    /// `global_sequence`.
    ///
    /// For `Maps`, resolves entity names through `scope_entities` and then
    /// falls back to callers re-filtering the stream index (this variant is
    /// intended for use by `StoreIndex::query` which performs that join).
    /// When called standalone it returns the entity set so the caller can join.
    ///
    /// For `Columnar`, delegates to [`ColumnarIndex::query_by_scope`].
    pub(crate) fn query_by_scope(&self, scope: &str) -> Vec<Arc<IndexEntry>> {
        self.query_route(self.capabilities().by_scope, EntryQuery::Scope(scope))
    }

    /// Return all entries whose kind falls in `category` (upper 4 bits),
    /// sorted by `global_sequence`.
    ///
    /// For `Maps`, iterates all kinds in `by_fact` and collects those matching
    /// the category. For `Columnar`, delegates to
    /// [`ColumnarIndex::query_by_category`].
    pub(crate) fn query_by_category(&self, category: u8) -> Vec<Arc<IndexEntry>> {
        self.query_route(
            self.capabilities().by_category,
            EntryQuery::Category(category),
        )
    }

    /// Return the set of entity strings registered under `scope` (Maps variant only).
    ///
    /// Returns `None` for the Columnar variant — callers should use
    /// [`query_by_scope`] instead.
    ///
    /// [`query_by_scope`]: ScanIndex::query_by_scope
    pub(crate) fn scope_entity_set(&self, scope: &str) -> Option<HashSet<Arc<str>>> {
        self.scope_entities.get(scope).map(|r| r.value().clone())
    }

    /// Discard all entries.  Called during index rebuild.
    pub(crate) fn clear(&self) {
        self.by_fact.clear();
        self.scope_entities.clear();
        if let Some(idx) = &self.soa {
            idx.clear();
        }
        if let Some(idx) = &self.entity_groups {
            idx.clear();
        }
        if let Some(idx) = &self.tiles64 {
            idx.clear();
        }
    }

    pub(crate) fn entity_generation(&self, entity: &str) -> Option<u64> {
        let projection = self.capabilities().projection;
        if !projection.entity_generation_fast_path {
            return None;
        }
        self.entity_groups
            .as_ref()
            .and_then(|idx| idx.entity_generation(entity))
    }

    pub(crate) fn cached_projection(
        &self,
        entity: &str,
        type_id: TypeId,
    ) -> Option<CachedProjectionSlot> {
        let projection = self.capabilities().projection;
        if !projection.cached_projection {
            return None;
        }
        self.entity_groups
            .as_ref()
            .and_then(|idx| idx.cached_projection(entity, type_id))
    }

    pub(crate) fn store_cached_projection(
        &self,
        entity: &str,
        type_id: TypeId,
        bytes: Vec<u8>,
        watermark: u64,
        cached_at_us: i64,
    ) -> bool {
        let projection = self.capabilities().projection;
        projection.cached_projection
            && self.entity_groups.as_ref().is_some_and(|idx| {
                idx.store_cached_projection(entity, type_id, bytes, watermark, cached_at_us)
            })
    }

    pub(crate) fn projection_candidates(
        &self,
        entity: &str,
        relevant_kinds: &[EventKind],
    ) -> Option<ProjectionCandidates> {
        let projection = self.capabilities().projection;
        if !projection.projection_candidates {
            return None;
        }
        self.entity_groups
            .as_ref()
            .and_then(|idx| idx.projection_candidates(entity, relevant_kinds))
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coordinate::Coordinate;
    use crate::event::{EventKind, HashChain};
    use crate::store::index::{DiskPos, IndexEntry};
    use std::sync::Arc;

    fn make_entry(kind: EventKind, seq: u64, entity: &str, scope: &str) -> Arc<IndexEntry> {
        let coord = Coordinate::new(entity, scope).expect("coord");
        Arc::new(IndexEntry {
            event_id: seq as u128,
            correlation_id: seq as u128,
            causation_id: None,
            coord,
            entity_id: crate::store::index::interner::InternId::sentinel(),
            scope_id: crate::store::index::interner::InternId::sentinel(),
            kind,
            wall_ms: seq * 1000,
            clock: u32::try_from(seq).expect("test seq fits u32"),
            dag_lane: 0,
            dag_depth: 0,
            hash_chain: HashChain::default(),
            disk_pos: DiskPos {
                segment_id: 0,
                offset: seq * 64,
                length: 64,
            },
            global_sequence: seq,
        })
    }

    const KIND_A: EventKind = EventKind::custom(0x1, 1);
    const KIND_B: EventKind = EventKind::custom(0x1, 2);

    // --- SoA ---

    #[test]
    fn soa_insert_and_query_by_kind() {
        let idx = ColumnarIndex::new_soa();
        for i in 0u64..10 {
            idx.insert(&make_entry(KIND_A, i, "e1", "s1"));
        }
        for i in 10u64..15 {
            idx.insert(&make_entry(KIND_B, i, "e2", "s1"));
        }
        let a = idx.query_by_kind(KIND_A);
        assert_eq!(a.len(), 10);
        // sorted by global_sequence
        for (i, e) in a.iter().enumerate() {
            assert_eq!(e.global_sequence, i as u64);
        }
        let b = idx.query_by_kind(KIND_B);
        assert_eq!(b.len(), 5);
    }

    #[test]
    fn soa_query_by_scope() {
        let idx = ColumnarIndex::new_soa();
        for i in 0u64..6 {
            idx.insert(&make_entry(KIND_A, i, "e1", "scope-x"));
        }
        for i in 6u64..10 {
            idx.insert(&make_entry(KIND_A, i, "e2", "scope-y"));
        }
        let x = idx.query_by_scope("scope-x");
        assert_eq!(x.len(), 6);
        let y = idx.query_by_scope("scope-y");
        assert_eq!(y.len(), 4);
        let z = idx.query_by_scope("scope-z");
        assert!(z.is_empty());
    }

    #[test]
    fn soa_clear() {
        let idx = ColumnarIndex::new_soa();
        for i in 0u64..5 {
            idx.insert(&make_entry(KIND_A, i, "e1", "s1"));
        }
        idx.clear();
        assert!(idx.query_by_kind(KIND_A).is_empty());
        assert!(idx.query_by_scope("s1").is_empty());
    }

    // --- AoSoA8 ---

    #[test]
    fn aosoa8_insert_spans_multiple_tiles() {
        let idx = ColumnarIndex::new_aosoa8();
        // 20 events of KIND_A → should fill 2 complete tiles + 1 partial (3 total)
        for i in 0u64..20 {
            idx.insert(&make_entry(KIND_A, i, "e1", "s1"));
        }
        let results = idx.query_by_kind(KIND_A);
        assert_eq!(results.len(), 20);
        for (i, e) in results.iter().enumerate() {
            assert_eq!(e.global_sequence, i as u64, "order must be preserved");
        }
    }

    #[test]
    fn aosoa8_interleaved_kinds() {
        let idx = ColumnarIndex::new_aosoa8();
        // Interleaved: push both kinds so tiles can't be pre-filled
        for i in 0u64..12 {
            idx.insert(&make_entry(KIND_A, i * 2, "ea", "s1"));
            idx.insert(&make_entry(KIND_B, i * 2 + 1, "eb", "s1"));
        }
        let a = idx.query_by_kind(KIND_A);
        let b = idx.query_by_kind(KIND_B);
        assert_eq!(a.len(), 12);
        assert_eq!(b.len(), 12);
    }

    #[test]
    fn aosoa8_query_by_scope() {
        let idx = ColumnarIndex::new_aosoa8();
        for i in 0u64..9 {
            idx.insert(&make_entry(KIND_A, i, "ent-a", "scope-alpha"));
        }
        for i in 9u64..14 {
            idx.insert(&make_entry(KIND_A, i, "ent-b", "scope-beta"));
        }
        let alpha = idx.query_by_scope("scope-alpha");
        assert_eq!(alpha.len(), 9);
        let beta = idx.query_by_scope("scope-beta");
        assert_eq!(beta.len(), 5);
    }

    #[test]
    fn aosoa8_with_tile_callback() {
        let idx = ColumnarIndex::new_aosoa8();
        for i in 0u64..8 {
            idx.insert(&make_entry(KIND_A, i, "e1", "s1"));
        }
        // First tile should be full with KIND_A
        let len = idx.with_tile8(0, |t| t.len).expect("should be AoSoA8");
        assert_eq!(len, 8);
    }

    // --- AoSoA16 ---

    #[test]
    fn aosoa16_basic() {
        let idx = ColumnarIndex::new_aosoa16();
        for i in 0u64..33 {
            idx.insert(&make_entry(KIND_A, i, "e1", "s1"));
        }
        assert_eq!(idx.query_by_kind(KIND_A).len(), 33);
    }

    #[test]
    fn aosoa16_with_tile_callback() {
        let idx = ColumnarIndex::new_aosoa16();
        for i in 0u64..16 {
            idx.insert(&make_entry(KIND_A, i, "e1", "s1"));
        }
        let len = idx.with_tile16(0, |t| t.len).expect("should be AoSoA16");
        assert_eq!(len, 16);
    }

    // --- AoSoA64 ---

    #[test]
    fn aosoa64_basic() {
        let idx = ColumnarIndex::new_aosoa64();
        for i in 0u64..130 {
            idx.insert(&make_entry(KIND_A, i, "e1", "s1"));
        }
        assert_eq!(idx.query_by_kind(KIND_A).len(), 130);
    }

    #[test]
    fn aosoa64_with_tile_callback() {
        let idx = ColumnarIndex::new_aosoa64();
        for i in 0u64..64 {
            idx.insert(&make_entry(KIND_A, i, "e1", "s1"));
        }
        let len = idx.with_tile64(0, |t| t.len).expect("should be AoSoA64");
        assert_eq!(len, 64);
    }

    // --- SoAoS ---

    #[test]
    fn soaos_insert_and_query_by_kind() {
        let idx = ColumnarIndex::new_soaos();
        for i in 0u64..10 {
            idx.insert(&make_entry(KIND_A, i, "e1", "s1"));
        }
        for i in 10u64..15 {
            idx.insert(&make_entry(KIND_B, i, "e2", "s1"));
        }
        assert_eq!(idx.query_by_kind(KIND_A).len(), 10);
        assert_eq!(idx.query_by_kind(KIND_B).len(), 5);
    }

    #[test]
    fn soaos_query_by_scope() {
        let idx = ColumnarIndex::new_soaos();
        for i in 0u64..8 {
            idx.insert(&make_entry(KIND_A, i, "e1", "scope-x"));
        }
        for i in 8u64..12 {
            idx.insert(&make_entry(KIND_A, i, "e2", "scope-y"));
        }
        let x = idx.query_by_scope("scope-x");
        assert_eq!(x.len(), 8);
        let y = idx.query_by_scope("scope-y");
        assert_eq!(y.len(), 4);
    }

    #[test]
    fn soaos_clear() {
        let idx = ColumnarIndex::new_soaos();
        for i in 0u64..5 {
            idx.insert(&make_entry(KIND_A, i, "e1", "s1"));
        }
        assert_eq!(idx.query_by_kind(KIND_A).len(), 5);
        idx.clear();
        assert_eq!(idx.query_by_kind(KIND_A).len(), 0);
    }

    // --- ScanIndex ---

    #[test]
    fn scan_index_maps_variant_insert_and_query() {
        let si = ScanIndex::for_config(&crate::store::IndexConfig {
            topology: crate::store::IndexTopology::aos(),
            incremental_projection: false,
            enable_checkpoint: true,
            enable_mmap_index: true,
        });
        for i in 0u64..7 {
            si.insert(&make_entry(KIND_A, i, "e1", "s1"));
        }
        let results = si.query_by_kind(KIND_A);
        assert_eq!(results.len(), 7);
    }

    #[test]
    fn scan_index_soa_variant_insert_and_query() {
        let si = ScanIndex::for_config(&crate::store::IndexConfig {
            topology: crate::store::IndexTopology::scan(),
            incremental_projection: false,
            enable_checkpoint: true,
            enable_mmap_index: true,
        });
        for i in 0u64..12 {
            si.insert(&make_entry(KIND_A, i, "e1", "s2"));
        }
        let results = si.query_by_kind(KIND_A);
        assert_eq!(results.len(), 12);
    }

    #[test]
    fn scan_capabilities_follow_topology_truth() {
        let cases = [
            (
                crate::store::IndexTopology::aos(),
                ScanCapabilities {
                    by_kind: ScanRoute::BaseAoS,
                    by_scope: ScanRoute::BaseAoS,
                    by_category: ScanRoute::BaseAoS,
                    projection: ProjectionSupport {
                        entity_generation_fast_path: false,
                        cached_projection: false,
                        projection_candidates: false,
                    },
                    topology_name: "aos",
                    tile_count: 0,
                },
            ),
            (
                crate::store::IndexTopology::scan(),
                ScanCapabilities {
                    by_kind: ScanRoute::SoA,
                    by_scope: ScanRoute::SoA,
                    by_category: ScanRoute::SoA,
                    projection: ProjectionSupport {
                        entity_generation_fast_path: false,
                        cached_projection: false,
                        projection_candidates: false,
                    },
                    topology_name: "scan",
                    tile_count: 0,
                },
            ),
            (
                crate::store::IndexTopology::entity_local(),
                ScanCapabilities {
                    by_kind: ScanRoute::SoAoS,
                    by_scope: ScanRoute::SoAoS,
                    by_category: ScanRoute::SoAoS,
                    projection: ProjectionSupport {
                        entity_generation_fast_path: true,
                        cached_projection: true,
                        projection_candidates: true,
                    },
                    topology_name: "entity-local",
                    tile_count: 0,
                },
            ),
            (
                crate::store::IndexTopology::tiled(),
                ScanCapabilities {
                    by_kind: ScanRoute::AoSoA64,
                    by_scope: ScanRoute::AoSoA64,
                    by_category: ScanRoute::AoSoA64,
                    projection: ProjectionSupport {
                        entity_generation_fast_path: false,
                        cached_projection: false,
                        projection_candidates: false,
                    },
                    topology_name: "tiled",
                    tile_count: 0,
                },
            ),
            (
                crate::store::IndexTopology::all(),
                ScanCapabilities {
                    by_kind: ScanRoute::SoA,
                    by_scope: ScanRoute::SoAoS,
                    by_category: ScanRoute::SoA,
                    projection: ProjectionSupport {
                        entity_generation_fast_path: true,
                        cached_projection: true,
                        projection_candidates: true,
                    },
                    topology_name: "all",
                    tile_count: 0,
                },
            ),
        ];

        for (topology, expected) in cases {
            let si = ScanIndex::for_config(&crate::store::IndexConfig {
                topology,
                incremental_projection: false,
                enable_checkpoint: true,
                enable_mmap_index: true,
            });
            assert_eq!(
                si.capabilities(),
                expected,
                "ScanCapabilities must be the single routing truth for `{}`",
                expected.topology_name
            );
        }
    }

    #[test]
    fn scan_index_aosoa8_variant() {
        let si = ScanIndex::for_config(&crate::store::IndexConfig {
            topology: crate::store::IndexTopology::tiled(),
            incremental_projection: false,
            enable_checkpoint: true,
            enable_mmap_index: true,
        });
        for i in 0u64..20 {
            si.insert(&make_entry(KIND_A, i, "e1", "s1"));
        }
        let results = si.query_by_kind(KIND_A);
        assert_eq!(results.len(), 20);
    }

    #[test]
    fn scan_index_maps_scope_entity_set() {
        let si = ScanIndex::for_config(&crate::store::IndexConfig {
            topology: crate::store::IndexTopology::aos(),
            incremental_projection: false,
            enable_checkpoint: true,
            enable_mmap_index: true,
        });
        si.insert(&make_entry(KIND_A, 0, "ent-1", "my-scope"));
        si.insert(&make_entry(KIND_A, 1, "ent-2", "my-scope"));
        let set = si
            .scope_entity_set("my-scope")
            .expect("should be Some for Maps");
        assert!(set.contains("ent-1" as &str));
        assert!(set.contains("ent-2" as &str));
    }

    #[test]
    fn scan_index_columnar_scope_entity_set_uses_base_aos_view() {
        let si = ScanIndex::for_config(&crate::store::IndexConfig {
            topology: crate::store::IndexTopology::scan(),
            incremental_projection: false,
            enable_checkpoint: true,
            enable_mmap_index: true,
        });
        si.insert(&make_entry(KIND_A, 0, "ent-1", "my-scope"));
        let set = si
            .scope_entity_set("my-scope")
            .expect("base AoS scope-entity map stays active across layouts");
        assert!(set.contains("ent-1" as &str));
    }

    #[test]
    fn scan_index_clear() {
        let si = ScanIndex::for_config(&crate::store::IndexConfig {
            topology: crate::store::IndexTopology::scan(),
            incremental_projection: false,
            enable_checkpoint: true,
            enable_mmap_index: true,
        });
        for i in 0u64..5 {
            si.insert(&make_entry(KIND_A, i, "e1", "s1"));
        }
        si.clear();
        assert!(si.query_by_kind(KIND_A).is_empty());
    }

    #[test]
    fn scan_index_soaos_variant() {
        let si = ScanIndex::for_config(&crate::store::IndexConfig {
            topology: crate::store::IndexTopology::entity_local(),
            incremental_projection: false,
            enable_checkpoint: true,
            enable_mmap_index: true,
        });
        for i in 0u64..10 {
            si.insert(&make_entry(KIND_A, i, "e1", "s1"));
        }
        assert_eq!(si.query_by_kind(KIND_A).len(), 10);
        assert_eq!(si.query_by_scope("s1").len(), 10);
        si.clear();
        assert!(si.query_by_kind(KIND_A).is_empty());
    }

    #[test]
    fn scan_capabilities_track_tile_count_for_tiled_views() {
        let si = ScanIndex::for_config(&crate::store::IndexConfig {
            topology: crate::store::IndexTopology::tiled(),
            incremental_projection: false,
            enable_checkpoint: true,
            enable_mmap_index: true,
        });
        for i in 0u64..130 {
            si.insert(&make_entry(KIND_A, i, "e1", "s1"));
        }
        let capabilities = si.capabilities();
        assert_eq!(capabilities.topology_name, "tiled");
        assert_eq!(capabilities.by_kind, ScanRoute::AoSoA64);
        assert_eq!(capabilities.by_scope, ScanRoute::AoSoA64);
        assert_eq!(capabilities.by_category, ScanRoute::AoSoA64);
        assert_eq!(capabilities.tile_count, 3);
        assert!(!capabilities.projection.cached_projection);
        assert!(!capabilities.projection.projection_candidates);
    }

    // --- Cross-layout oracle: all layouts must agree on query results ---
    //
    // This test is the correctness contract that makes the AoSoA64 SIMD
    // specialization (Step 4) safe to add: any specialized executor must
    // produce the same output as SoA on the same corpus.

    const KIND_C: EventKind = EventKind::custom(0x2, 1); // different category from KIND_A/KIND_B

    fn build_oracle_corpus() -> Vec<Arc<IndexEntry>> {
        // 20 KIND_A across two entities + 10 KIND_B + 5 KIND_C, two scopes.
        // Interleaved insertion to stress tile bucketing in AoSoA.
        let mut entries = Vec::new();
        let mut seq = 0u64;
        for _ in 0..10 {
            entries.push(make_entry(KIND_A, seq, "entity-alpha", "scope-one"));
            seq += 1;
            entries.push(make_entry(KIND_B, seq, "entity-beta", "scope-one"));
            seq += 1;
        }
        for _ in 0..10 {
            entries.push(make_entry(KIND_A, seq, "entity-gamma", "scope-two"));
            seq += 1;
            entries.push(make_entry(KIND_C, seq, "entity-gamma", "scope-two"));
            seq += 1;
        }
        entries
    }

    fn ids(v: &[Arc<IndexEntry>]) -> Vec<u64> {
        v.iter().map(|e| e.global_sequence).collect()
    }

    #[test]
    fn all_layouts_agree_on_by_kind() {
        let corpus = build_oracle_corpus();
        let soa = ColumnarIndex::new_soa();
        let aosoa64 = ColumnarIndex::new_aosoa64();
        let soaos = ColumnarIndex::new_soaos();
        for entry in &corpus {
            soa.insert(entry);
            aosoa64.insert(entry);
            soaos.insert(entry);
        }
        for kind in [KIND_A, KIND_B, KIND_C] {
            let reference = ids(&soa.query_by_kind(kind));
            assert_eq!(
                ids(&aosoa64.query_by_kind(kind)),
                reference,
                "AoSoA64 by_kind({kind:?}) must match SoA"
            );
            assert_eq!(
                ids(&soaos.query_by_kind(kind)),
                reference,
                "SoAoS by_kind({kind:?}) must match SoA"
            );
        }
    }

    #[test]
    fn all_layouts_agree_on_by_category() {
        let corpus = build_oracle_corpus();
        let soa = ColumnarIndex::new_soa();
        let aosoa64 = ColumnarIndex::new_aosoa64();
        let soaos = ColumnarIndex::new_soaos();
        for entry in &corpus {
            soa.insert(entry);
            aosoa64.insert(entry);
            soaos.insert(entry);
        }
        // KIND_A and KIND_B share category 0x1; KIND_C has category 0x2.
        for category in [0x1u8, 0x2u8] {
            let reference = ids(&soa.query_by_category(category));
            assert_eq!(
                ids(&aosoa64.query_by_category(category)),
                reference,
                "AoSoA64 by_category(0x{category:x}) must match SoA"
            );
            assert_eq!(
                ids(&soaos.query_by_category(category)),
                reference,
                "SoAoS by_category(0x{category:x}) must match SoA"
            );
        }
    }

    #[test]
    fn all_layouts_agree_on_by_scope() {
        let corpus = build_oracle_corpus();
        let soa = ColumnarIndex::new_soa();
        let aosoa64 = ColumnarIndex::new_aosoa64();
        let soaos = ColumnarIndex::new_soaos();
        for entry in &corpus {
            soa.insert(entry);
            aosoa64.insert(entry);
            soaos.insert(entry);
        }
        for scope in ["scope-one", "scope-two", "scope-missing"] {
            let reference = ids(&soa.query_by_scope(scope));
            assert_eq!(
                ids(&aosoa64.query_by_scope(scope)),
                reference,
                "AoSoA64 by_scope({scope:?}) must match SoA"
            );
            assert_eq!(
                ids(&soaos.query_by_scope(scope)),
                reference,
                "SoAoS by_scope({scope:?}) must match SoA"
            );
        }
    }
}
