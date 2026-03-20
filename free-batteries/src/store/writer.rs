#[cfg(feature = "exponential-backoff")]
compile_error!(
    "Red flag: only Once and Bounded restart policies. \
     Exponential backoff belongs in the product's supervisor, not here. \
     See: SPEC.md ## RED FLAGS."
);

use crate::coordinate::{Coordinate, DagPosition};
use crate::event::{Event, EventKind, HashChain};
use crate::store::index::{StoreIndex, IndexEntry, DiskPos};
use crate::store::segment::{self, Segment, Active, FramePayload};
use crate::store::{StoreConfig, StoreError, AppendReceipt};
use flume::{Sender, Receiver, TrySendError};
use parking_lot::Mutex;
use std::sync::Arc;
use tracing::{debug, info, warn, trace};

/// WriterCommand: messages sent to the background writer thread via flume.
/// All respond channels: flume::Sender — sync send from writer, async recv from caller.
/// [SPEC:src/store/writer.rs]
pub(crate) enum WriterCommand {
    Append {
        entity: Arc<str>,
        scope: Arc<str>,
        event: Event<Vec<u8>>,  // pre-serialized payload as msgpack bytes
        kind: EventKind,
        correlation_id: u128,
        causation_id: Option<u128>,
        respond: Sender<Result<AppendReceipt, StoreError>>,
    },
    Sync {
        respond: Sender<Result<(), StoreError>>,
    },
    Shutdown {
        respond: Sender<Result<(), StoreError>>,
    },
}

/// WriterHandle: owned by Store. Communicates with the background thread.
pub(crate) struct WriterHandle {
    pub tx: Sender<WriterCommand>,
    pub subscribers: Arc<SubscriberList>,
    thread: Option<std::thread::JoinHandle<()>>,
}

/// SubscriberList: push-based notification fanout via flume channels.
/// [SPEC:src/store/writer.rs — try_send pattern]
pub(crate) struct SubscriberList {
    senders: Mutex<Vec<Sender<Notification>>>,
}

/// Notification: lightweight event summary pushed to subscribers.
/// Must derive Clone (used in try_send broadcast loop).
/// [SPEC:src/store/writer.rs — Notification struct]
#[derive(Clone, Debug)]
pub struct Notification {
    pub event_id: u128,
    pub correlation_id: u128,
    pub causation_id: Option<u128>,
    pub coord: Coordinate,
    pub kind: EventKind,
    pub sequence: u64,
}

/// RestartPolicy: how the writer recovers from panics.
/// [SPEC:src/store/writer.rs — RestartPolicy]
/// EXACTLY two variants. Adding a third violates the RED FLAGS.
#[derive(Clone, Debug)]
pub enum RestartPolicy {
    Once,
    Bounded { max_restarts: u32, within_ms: u64 },
}

impl Default for RestartPolicy {
    fn default() -> Self { Self::Once }
}

impl SubscriberList {
    pub(crate) fn new() -> Self {
        Self { senders: Mutex::new(Vec::new()) }
    }

    /// Subscribe: create a new bounded channel, store the sender, return the receiver.
    pub(crate) fn subscribe(&self, capacity: usize) -> Receiver<Notification> {
        let (tx, rx) = flume::bounded(capacity);
        self.senders.lock().push(tx);
        rx
    }

    /// Broadcast: try_send to all, retain on Ok or Full, prune on Disconnected.
    /// NEVER use blocking send() — one slow subscriber must not block the writer.
    /// [DEP:flume::Sender::try_send] → Result<(), TrySendError<T>>
    /// [DEP:flume::TrySendError::Full] / [DEP:flume::TrySendError::Disconnected]
    pub(crate) fn broadcast(&self, notif: Notification) {
        let mut guard = self.senders.lock();
        guard.retain(|tx| match tx.try_send(notif.clone()) {
            Ok(()) => true,
            Err(TrySendError::Full(_)) => true,
            Err(TrySendError::Disconnected(_)) => false,
        });
    }
}

impl WriterHandle {
    /// Spawn the background writer thread.
    /// [SPEC:src/store/writer.rs — "free-batteries-writer" thread]
    pub(crate) fn spawn(
        config: Arc<StoreConfig>,
        index: Arc<StoreIndex>,
        subscribers: Arc<SubscriberList>,
    ) -> Result<Self, StoreError> {
        let (tx, rx) = flume::bounded::<WriterCommand>(config.writer_channel_capacity);
        let subs = Arc::clone(&subscribers);
        let cfg = Arc::clone(&config);
        let idx = Arc::clone(&index);

        let thread = std::thread::Builder::new()
            .name("free-batteries-writer".into())
            .spawn(move || {
                writer_loop(rx, cfg, idx, subs);
            })
            .map_err(|e| StoreError::Io(e))?;

        Ok(Self { tx, subscribers, thread: Some(thread) })
    }

    /// NOTE: No send_append() method here. Store::append() and Store::append_reaction()
    /// in store/mod.rs create one-shot flume channels and send WriterCommands directly
    /// via self.writer.tx.send(). This avoids an unnecessary abstraction layer.
    /// WriterHandle.tx is pub(crate) for direct access. [SPEC:INVARIANTS item 4]
}

/// The writer's main loop. Runs on the background thread.
fn writer_loop(
    rx: Receiver<WriterCommand>,
    config: Arc<StoreConfig>,
    index: Arc<StoreIndex>,
    subscribers: Arc<SubscriberList>,
) {
    let data_dir = &config.data_dir;
    /// Initialize: create data_dir if not exists, find latest segment or create first.
    std::fs::create_dir_all(data_dir).expect("create data dir");
    let mut segment_id: u64 = find_latest_segment_id(data_dir).unwrap_or(0) + 1;
    let mut active_segment = Segment::<Active>::create(data_dir, segment_id)
        .expect("create initial segment");
    let mut events_since_sync: u32 = 0;

    /// Main loop: recv commands, dispatch.
    for cmd in rx.iter() {
        match cmd {
            WriterCommand::Append { entity, scope, event, kind,
                                    correlation_id, causation_id, respond } => {
                let result = handle_append(
                    &entity, &scope, event, kind, correlation_id, causation_id,
                    &index, &mut active_segment, &mut segment_id,
                    &config, &subscribers,
                );
                // Respond to caller. Ignore send error (caller may have dropped).
                let _ = respond.send(result);

                events_since_sync += 1;
                if events_since_sync >= config.sync_every_n_events {
                    let _ = active_segment.sync();
                    events_since_sync = 0;
                }
            }
            WriterCommand::Sync { respond } => {
                let result = active_segment.sync().map_err(|e| e);
                let _ = respond.send(result);
                events_since_sync = 0;
            }
            WriterCommand::Shutdown { respond } => {
                // Drain up to shutdown_drain_limit queued commands.
                // [SPEC:src/store/writer.rs — Shutdown drain semantics]
                let mut drained = 0;
                while drained < config.shutdown_drain_limit {
                    match rx.try_recv() {
                        Ok(WriterCommand::Append { entity, scope, event, kind,
                                                   correlation_id, causation_id, respond: r }) => {
                            let result = handle_append(
                                &entity, &scope, event, kind, correlation_id, causation_id,
                                &index, &mut active_segment, &mut segment_id,
                                &config, &subscribers,
                            );
                            let _ = r.send(result);
                            drained += 1;
                        }
                        Ok(WriterCommand::Shutdown { respond: r }) => {
                            let _ = r.send(Ok(()));
                        }
                        Ok(WriterCommand::Sync { respond: r }) => {
                            let _ = r.send(active_segment.sync());
                        }
                        Err(_) => break, // channel empty
                    }
                }
                let _ = active_segment.sync();
                let _ = respond.send(Ok(()));
                return; // exit writer loop
            }
        }
    }
}

/// The 10-step commit protocol.
/// [SPEC:src/store/writer.rs — handle_append]
fn handle_append(
    entity: &Arc<str>,
    scope: &Arc<str>,
    mut event: Event<Vec<u8>>,
    kind: EventKind,
    correlation_id: u128,
    causation_id: Option<u128>,
    index: &StoreIndex,
    active_segment: &mut Segment<Active>,
    segment_id: &mut u64,
    config: &StoreConfig,
    subscribers: &SubscriberList,
) -> Result<AppendReceipt, StoreError> {

    /// STEP 1: Acquire per-entity lock.
    /// [SPEC:IMPLEMENTATION NOTES item 5 — DashMap guard lifetimes]
    /// Clone the Arc<Mutex> OUT of DashMap, drop the DashMap entry guard,
    /// THEN lock the Mutex. Never hold DashMap Ref across the commit.
    let lock = index.entity_locks.entry(entity.clone())
        .or_insert_with(|| Arc::new(parking_lot::Mutex::new(())))
        .clone();
    let _entity_guard = lock.lock();
    debug!(entity = %entity, "entity lock acquired");

    /// STEP 2: Get prev_hash from index (or [0u8;32] for genesis).
    /// Clone the value out of the DashMap Ref immediately.
    let prev_hash = index.get_latest(entity)
        .map(|e| e.hash_chain.event_hash)
        .unwrap_or([0u8; 32]);

    /// STEP 3: Compute sequence (latest.clock + 1, or 0).
    let clock = index.get_latest(entity)
        .map(|e| e.clock + 1)
        .unwrap_or(0);

    /// STEP 4: Set event header position.
    let position = DagPosition::child(clock);
    event.header.position = position;
    event.header.event_kind = kind;
    event.header.correlation_id = correlation_id;
    event.header.causation_id = causation_id;

    /// STEP 5: Compute blake3 hash, set hash chain (or skip if feature off).
    /// [SPEC:INVARIANTS item 5 — blake3 only]
    let payload_for_hash = &event.payload; // pre-serialized bytes
    #[cfg(feature = "blake3")]
    let event_hash = crate::event::hash::compute_hash(payload_for_hash);
    #[cfg(not(feature = "blake3"))]
    let event_hash = [0u8; 32];

    event.hash_chain = Some(HashChain { prev_hash, event_hash });

    /// STEP 6: Serialize to MessagePack + CRC32 frame.
    /// [SPEC:WIRE FORMAT DECISIONS — rmp_serde::to_vec_named() ALWAYS]
    let frame_payload = FramePayload {
        event: event.clone(),
        entity: entity.to_string(),
        scope: scope.to_string(),
    };
    let frame = segment::frame_encode(&frame_payload)?;

    /// STEP 7: Check segment rotation.
    if active_segment.needs_rotation(config.segment_max_bytes) {
        active_segment.sync()?;
        let old = std::mem::replace(
            active_segment,
            Segment::<Active>::create(&config.data_dir, *segment_id + 1)?,
        );
        let _sealed = old.seal();
        *segment_id += 1;
        info!(segment_id = *segment_id, "segment rotated");
    }

    /// STEP 8: Write frame to segment file.
    let offset = active_segment.write_frame(&frame)?;
    trace!(offset = offset, len = frame.len(), "frame written");

    /// STEP 9: Update index.
    let global_seq = index.global_sequence();
    let disk_pos = DiskPos {
        segment_id: *segment_id,
        offset,
        length: frame.len() as u32,
    };
    let entry = IndexEntry {
        event_id: event.header.event_id,
        correlation_id,
        causation_id,
        coord: Coordinate::new(entity.as_ref(), scope.as_ref())
            .map_err(StoreError::Coordinate)?,
        kind,
        clock,
        hash_chain: event.hash_chain.clone().unwrap_or_default(),
        disk_pos: disk_pos.clone(),
        global_sequence: global_seq,
    };
    index.insert(entry);
    debug!(event_id = %event.header.event_id, clock = clock, "append committed");

    /// STEP 10: Broadcast notification to subscribers.
    subscribers.broadcast(Notification {
        event_id: event.header.event_id,
        correlation_id,
        causation_id,
        coord: Coordinate::new(entity.as_ref(), scope.as_ref())
            .map_err(StoreError::Coordinate)?,
        kind,
        sequence: global_seq,
    });

    Ok(AppendReceipt {
        event_id: event.header.event_id,
        sequence: global_seq,
        disk_pos,
    })
}

/// Find the latest segment ID by scanning data_dir for .fbat files.
fn find_latest_segment_id(dir: &std::path::Path) -> Option<u64> {
    std::fs::read_dir(dir).ok()?
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let name = e.file_name();
            let name = name.to_str()?;
            if name.ends_with(".fbat") {
                name.trim_end_matches(".fbat").parse::<u64>().ok()
            } else { None }
        })
        .max()
}
