mod ancestors;
mod config;
mod contracts;
pub mod cursor;
mod error;
pub mod index;
mod maintenance;
pub mod projection;
mod projection_flow;
pub mod reader;
#[cfg(test)]
mod runtime_contracts;
pub mod segment;
pub mod stats;
pub mod subscription;
#[cfg(feature = "test-support")]
mod test_support;
pub mod writer;

pub use config::{StoreConfig, SyncMode};
pub use contracts::{
    AppendOptions, AppendReceipt, CompactionConfig, CompactionStrategy, RetentionPredicate,
};
pub use cursor::Cursor;
pub use error::StoreError;
pub use index::{ClockKey, DiskPos, IndexEntry};
#[cfg(feature = "lmdb")]
pub use projection::LmdbCache;
#[cfg(feature = "redb")]
pub use projection::RedbCache;
pub use projection::{CacheCapabilities, CacheMeta, Freshness, NoCache, ProjectionCache};
pub use stats::{StoreDiagnostics, StoreStats};
pub use subscription::Subscription;
pub use writer::{Notification, RestartPolicy};

use crate::coordinate::{Coordinate, KindFilter, Region};
use crate::event::{Event, EventHeader, EventKind, EventSourced, StoredEvent};
#[cfg(test)]
pub(crate) use config::now_us;
use contracts::checked_payload_len;
use index::StoreIndex;
use reader::Reader;
use serde::Serialize;
use std::sync::Arc;
use writer::{AppendGuards, SubscriberList, WriterCommand, WriterHandle};
// ProjectionCache re-exported above via pub use, no separate use needed.

/// Store: the runtime. Sync API. Send + Sync.
/// [SPEC:src/store/mod.rs]
/// Invariant 2: ALL METHODS ARE SYNC. No .await anywhere.
// async-store is not a declared feature — suppress cfg warning for this guard
#[allow(unexpected_cfgs)]
#[cfg(feature = "async-store")]
compile_error!("INVARIANT 2: Store API is sync. Use spawn_blocking or flume recv_async.");

pub struct Store {
    index: Arc<StoreIndex>,
    reader: Arc<Reader>,
    cache: Box<dyn ProjectionCache>,
    writer: WriterHandle,
    config: Arc<StoreConfig>,
}

impl Store {
    /// Open a store with default config at `./batpak-data`.
    /// Sugar over `Store::open(StoreConfig::new("./batpak-data"))`.
    /// [SPEC:src/store/mod.rs — Store::open_default]
    pub fn open_default() -> Result<Self, StoreError> {
        Self::open(StoreConfig::new("./batpak-data"))
    }

    /// Open a store at the given config's data directory. Creates the directory if absent.
    /// Uses `NoCache` for projection (no external cache backend).
    pub fn open(config: StoreConfig) -> Result<Self, StoreError> {
        Self::open_with_cache(config, Box::new(NoCache))
    }

    /// Open a store with a custom projection cache backend.
    /// Use `RedbCache` or `LmdbCache` (feature-gated) for cache-accelerated `project()` calls.
    pub fn open_with_cache(
        config: StoreConfig,
        cache: Box<dyn ProjectionCache>,
    ) -> Result<Self, StoreError> {
        std::fs::create_dir_all(&config.data_dir)?;
        let config = Arc::new(config);
        let index = Arc::new(StoreIndex::new());
        let reader = Arc::new(Reader::new(config.data_dir.clone(), config.fd_budget));

        // Cold start: scan all segments, rebuild index.
        // [SPEC:IMPLEMENTATION NOTES item 2 — segment naming, alphabetical scan]
        let mut entries: Vec<std::fs::DirEntry> = std::fs::read_dir(&config.data_dir)?
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.path()
                    .extension()
                    .map(|ext| ext == segment::SEGMENT_EXTENSION)
                    .unwrap_or(false)
            })
            .collect();
        entries.sort_by_key(|e| e.file_name());

        for dir_entry in &entries {
            let scanned = reader.scan_segment_index(&dir_entry.path())?;
            for se in scanned {
                let coord = Coordinate::new(&se.entity, &se.scope)?;
                let clock = se.header.position.sequence;
                let entry = IndexEntry {
                    event_id: se.header.event_id,
                    correlation_id: se.header.correlation_id,
                    causation_id: se.header.causation_id,
                    coord,
                    kind: se.header.event_kind,
                    wall_ms: se.header.position.wall_ms,
                    clock,
                    hash_chain: se.hash_chain,
                    disk_pos: DiskPos {
                        segment_id: se.segment_id,
                        offset: se.offset,
                        length: se.length,
                    },
                    global_sequence: index.global_sequence(),
                };
                index.insert(entry);
            }
        }

        let subscribers = Arc::new(SubscriberList::new());
        let writer = WriterHandle::spawn(&config, &index, &subscribers)?;

        Ok(Self {
            index,
            reader,
            cache,
            writer,
            config,
        })
    }

    /// WRITE: append a new root-cause event.
    /// correlation_id defaults to event_id (self-correlated). causation_id = None.
    pub fn append(
        &self,
        coord: &Coordinate,
        kind: EventKind,
        payload: &impl Serialize,
    ) -> Result<AppendReceipt, StoreError> {
        tracing::debug!(
            target: "batpak::flow",
            flow = "append",
            entity = coord.entity(),
            scope = coord.scope(),
            event_kind = kind.type_id()
        );
        let payload_bytes = rmp_serde::to_vec_named(payload)
            .map_err(|e| StoreError::Serialization(e.to_string()))?;
        let payload_len = checked_payload_len(&payload_bytes)?;
        let event_id = crate::id::generate_v7_id();
        let header = EventHeader::new(
            event_id,
            event_id,
            None, // correlation = self, causation = root
            self.config.now_us(),
            crate::coordinate::DagPosition::root(),
            payload_len,
            kind,
        );
        let event = Event::new(header, payload_bytes);

        let (tx, rx) = flume::bounded(1);
        self.writer
            .tx
            .send(WriterCommand::Append {
                entity: coord.entity_arc(),
                scope: coord.scope_arc(),
                event: Box::new(event),
                kind,
                guards: AppendGuards {
                    correlation_id: event_id,
                    causation_id: None,
                    expected_sequence: None,
                    idempotency_key: None,
                },
                respond: tx,
            })
            .map_err(|_| StoreError::WriterCrashed)?;

        rx.recv().map_err(|_| StoreError::WriterCrashed)?
    }

    /// WRITE: append a reaction (caused by another event).
    pub fn append_reaction(
        &self,
        coord: &Coordinate,
        kind: EventKind,
        payload: &impl Serialize,
        correlation_id: u128,
        causation_id: u128,
    ) -> Result<AppendReceipt, StoreError> {
        tracing::debug!(
            target: "batpak::flow",
            flow = "append_reaction",
            entity = coord.entity(),
            scope = coord.scope(),
            correlation_id = format_args!("{correlation_id:032x}"),
            causation_id = format_args!("{causation_id:032x}")
        );
        let payload_bytes = rmp_serde::to_vec_named(payload)
            .map_err(|e| StoreError::Serialization(e.to_string()))?;
        let payload_len = checked_payload_len(&payload_bytes)?;
        let event_id = crate::id::generate_v7_id();
        let header = EventHeader::new(
            event_id,
            correlation_id,
            Some(causation_id),
            self.config.now_us(),
            crate::coordinate::DagPosition::root(),
            payload_len,
            kind,
        );
        let event = Event::new(header, payload_bytes);

        let (tx, rx) = flume::bounded(1);
        self.writer
            .tx
            .send(WriterCommand::Append {
                entity: coord.entity_arc(),
                scope: coord.scope_arc(),
                event: Box::new(event),
                kind,
                guards: AppendGuards {
                    correlation_id,
                    causation_id: Some(causation_id),
                    expected_sequence: None,
                    idempotency_key: None,
                },
                respond: tx,
            })
            .map_err(|_| StoreError::WriterCrashed)?;

        rx.recv().map_err(|_| StoreError::WriterCrashed)?
    }

    /// READ: get a single event by ID.
    pub fn get(&self, event_id: u128) -> Result<StoredEvent<serde_json::Value>, StoreError> {
        let entry = self
            .index
            .get_by_id(event_id)
            .ok_or(StoreError::NotFound(event_id))?;
        self.reader.read_entry(&entry.disk_pos)
    }

    /// READ: query by Region.
    pub fn query(&self, region: &Region) -> Vec<IndexEntry> {
        self.index.query(region)
    }

    /// READ: walk hash chain ancestors. [SPEC:IMPLEMENTATION NOTES item 3]
    /// When blake3 is enabled, follows the hash chain (event_hash -> prev_hash).
    /// When blake3 is disabled, all hashes are `[0u8;32]` so hash-based walking
    /// is impossible. Falls back to clock-ordered traversal (descending clock).
    pub fn walk_ancestors(
        &self,
        event_id: u128,
        limit: usize,
    ) -> Vec<StoredEvent<serde_json::Value>> {
        ancestors::walk_ancestors(self, event_id, limit)
    }

    /// PROJECT: reconstruct typed state from events, with cache support.
    /// [SPEC:src/store/mod.rs — Projection Flow]
    pub fn project<T>(&self, entity: &str, freshness: &Freshness) -> Result<Option<T>, StoreError>
    where
        T: EventSourced<serde_json::Value> + serde::Serialize + serde::de::DeserializeOwned,
    {
        projection_flow::project(self, entity, freshness)
    }

    /// SUBSCRIBE: push-based, lossy.
    pub fn subscribe(&self, region: &Region) -> Subscription {
        let rx = self
            .writer
            .subscribers
            .subscribe(self.config.broadcast_capacity);
        Subscription::new(rx, region.clone())
    }

    /// CURSOR: pull-based, guaranteed delivery.
    pub fn cursor(&self, region: &Region) -> Cursor {
        Cursor::new(region.clone(), Arc::clone(&self.index))
    }

    /// CONVENIENCE: sugar over index.stream() for exact entity match.
    /// Unlike Region::entity() (prefix match), this returns events for
    /// exactly the named entity — "entity:1" does NOT match "entity:10".
    pub fn stream(&self, entity: &str) -> Vec<IndexEntry> {
        self.index.stream(entity)
    }
    pub fn by_scope(&self, scope: &str) -> Vec<IndexEntry> {
        self.query(&Region::scope(scope))
    }
    pub fn by_fact(&self, kind: EventKind) -> Vec<IndexEntry> {
        self.query(&Region::all().with_fact(KindFilter::Exact(kind)))
    }

    /// REACT: spawn a background thread running the subscribe→react→append loop.
    /// Returns a JoinHandle. The thread runs until the store is dropped (subscription closes).
    /// \[SPEC:src/event/sourcing.rs — Reactive\<P\> glue pattern\]
    pub fn react_loop<R>(
        self: &Arc<Self>,
        region: &Region,
        reactor: R,
    ) -> Result<std::thread::JoinHandle<()>, StoreError>
    where
        R: crate::event::sourcing::Reactive<serde_json::Value> + Send + 'static,
    {
        let store = Arc::clone(self);
        let sub = self.subscribe(region);
        std::thread::Builder::new()
            .name("batpak-reactor".into())
            .spawn(move || {
                while let Some(notif) = sub.recv() {
                    let stored = match store.get(notif.event_id) {
                        Ok(s) => s,
                        Err(e) => {
                            tracing::warn!(
                                "react_loop: failed to get event {}: {e}",
                                notif.event_id
                            );
                            continue;
                        }
                    };
                    for (coord, kind, payload) in reactor.react(&stored.event) {
                        if let Err(e) = store.append_reaction(
                            &coord,
                            kind,
                            &payload,
                            notif.correlation_id,
                            notif.event_id,
                        ) {
                            tracing::warn!("react_loop: failed to append reaction: {e}");
                        }
                    }
                }
            })
            .map_err(StoreError::Io)
    }

    /// WRITE: append with CAS, idempotency, custom correlation/causation.
    /// CAS and idempotency checks execute inside the writer thread under
    /// the entity lock — no TOCTOU race between check and commit.
    pub fn append_with_options(
        &self,
        coord: &Coordinate,
        kind: EventKind,
        payload: &impl Serialize,
        opts: AppendOptions,
    ) -> Result<AppendReceipt, StoreError> {
        tracing::debug!(
            target: "batpak::flow",
            flow = "append_with_options",
            entity = coord.entity(),
            scope = coord.scope(),
            has_cas = opts.expected_sequence.is_some(),
            has_idempotency = opts.idempotency_key.is_some()
        );
        let payload_bytes = rmp_serde::to_vec_named(payload)
            .map_err(|e| StoreError::Serialization(e.to_string()))?;
        let payload_len = checked_payload_len(&payload_bytes)?;
        let event_id = opts
            .idempotency_key
            .unwrap_or_else(crate::id::generate_v7_id);
        let correlation_id = opts.correlation_id.unwrap_or(event_id);
        let causation_id = opts.causation_id;
        let header = EventHeader::new(
            event_id,
            correlation_id,
            causation_id,
            self.config.now_us(),
            crate::coordinate::DagPosition::root(),
            payload_len,
            kind,
        )
        .with_flags(opts.flags);
        let event = Event::new(header, payload_bytes);

        let (tx, rx) = flume::bounded(1);
        self.writer
            .tx
            .send(WriterCommand::Append {
                entity: coord.entity_arc(),
                scope: coord.scope_arc(),
                event: Box::new(event),
                kind,
                guards: AppendGuards {
                    correlation_id,
                    causation_id,
                    expected_sequence: opts.expected_sequence,
                    idempotency_key: opts.idempotency_key,
                },
                respond: tx,
            })
            .map_err(|_| StoreError::WriterCrashed)?;

        rx.recv().map_err(|_| StoreError::WriterCrashed)?
    }

    /// WRITE: apply a typestate transition — extracts kind+payload, delegates to append.
    pub fn apply_transition<From, To, P: Serialize>(
        &self,
        coord: &Coordinate,
        transition: crate::typestate::transition::Transition<From, To, P>,
    ) -> Result<AppendReceipt, StoreError> {
        let kind = transition.kind();
        let payload = transition.into_payload();
        self.append(coord, kind, &payload)
    }

    /// LIFECYCLE
    pub fn sync(&self) -> Result<(), StoreError> {
        maintenance::sync(self)
    }

    /// Snapshot the current index to a destination directory.
    pub fn snapshot(&self, dest: &std::path::Path) -> Result<(), StoreError> {
        maintenance::snapshot(self, dest)
    }

    /// Compact: merge sealed segments, optionally filtering events.
    /// Returns the number of segments removed and bytes reclaimed.
    /// The active (currently-written) segment is never touched.
    ///
    /// **IMPORTANT**: compact() rebuilds the in-memory index from disk.
    /// Appends that arrive during compaction are safe (they go to the active
    /// segment which is not compacted), but the index rebuild syncs the writer
    /// before and after to minimize the window for stale index state.
    /// For maximum safety, avoid high-throughput appends during compaction.
    pub fn compact(
        &self,
        config: &CompactionConfig,
    ) -> Result<segment::CompactionResult, StoreError> {
        maintenance::compact(self, config)
    }

    pub fn close(self) -> Result<(), StoreError> {
        maintenance::close(self)
    }

    /// DIAGNOSTICS
    pub fn stats(&self) -> StoreStats {
        maintenance::stats(self)
    }

    pub fn diagnostics(&self) -> StoreDiagnostics {
        maintenance::diagnostics(self)
    }
}

/// Safety net: if Store is dropped without calling close(), send a best-effort
/// Shutdown to the writer thread and wait briefly for it to drain pending events.
/// close(self) is still the preferred explicit path for guaranteed clean shutdown.
impl Drop for Store {
    fn drop(&mut self) {
        let (tx, rx) = flume::bounded(1);
        if self
            .writer
            .tx
            .send(WriterCommand::Shutdown { respond: tx })
            .is_ok()
        {
            // Wait up to 100ms for the writer to drain pending events.
            // This prevents data loss when Store is dropped without close().
            let _ = rx.recv_timeout(std::time::Duration::from_millis(100));
        }
    }
}
