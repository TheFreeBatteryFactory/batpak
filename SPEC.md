# free-b (batteries) — Airgapped Build Spec

One document. No overrides. No layers. Every decision is final. A cloud agent with zero prior context builds the entire library from this file.

---

## INVARIANTS (check every decision against these — they override everything)

```
1. NO TOKIO IN REQUIRED DEPS. The library is runtime-agnostic.
   tokio stays in dev-deps only. Fan-out uses Vec<flume::Sender>.

2. STORE API IS SYNC. One set of methods. Bisync is a CHANNEL property,
   not an API property. The Store doesn't know if its caller is sync or async.

3. NO PRODUCT CONCEPTS IN LIBRARY CODE. No "trajectory", "scope",
   "artifact", "agent", "tenant", "turn", "note". Library vocabulary:
   coordinate, entity, event, outcome, gate, region, transition.

4. NO SPECULATIVE ABSTRACTIONS. No trait until there's a second impl.
   No generic param until there's a second type. No module until there's
   enough code. Test: "Does removing this break working code? If no, skip."

5. BLAKE3 IS THE ONLY HASH. No HashAlgorithm enum. No EventVerifier trait.
   One function: compute_hash(bytes) -> [u8; 32], behind feature = "blake3".
```

---

## WHAT V1 IS

```
free-batteries v1 is a coordinate-addressed append-only causal log with
typestate-aware transitions and projection replay.

It IS:  a library for building event-sourced state machines over coordinate spaces.
It is NOT:  multi-lane, distributed, a transformer-gate algebra, or a generic
            storage trait ecosystem. It is DAG-ready, not DAG-complete.
```

---

## SIX PRINCIPLES

```
1. Everything is somewhere.        -> coordinate/
2. Everything has an outcome.      -> outcome/
3. Allowed = constructible.        -> guard/
4. Writing is executing.           -> store/
5. Phase tells you what you can do -> pipeline/
6. Structure survives transform.   -> (functor laws on Outcome, tested via proptest)
```

---

## FILE TREE

```
free-batteries/
├── .cargo/config.toml
├── .config/nextest.toml
├── .github/workflows/ci.yml
├── .gitignore
├── Cargo.toml
├── CHANGELOG.md
├── LICENSE-APACHE
├── LICENSE-MIT
├── README.md
├── clippy.toml
├── justfile
├── rust-toolchain.toml
│
├── src/
│   ├── lib.rs
│   ├── prelude.rs
│   │
│   ├── coordinate/
│   │   ├── mod.rs            # Coordinate (Arc<str>), Region, CoordinateError, EventKindFilter
│   │   └── position.rs       # DagPosition (depth, lane, sequence)
│   │
│   ├── outcome/
│   │   ├── mod.rs            # Outcome<T> — 6 variants + all combinators
│   │   ├── error.rs          # OutcomeError, ErrorKind (9 + Custom(u16))
│   │   ├── combine.rs        # zip, join_all, join_any
│   │   └── wait.rs           # WaitCondition, CompensationAction
│   │
│   ├── event/
│   │   ├── mod.rs            # Event<P>, StoredEvent<P>
│   │   ├── header.rs         # EventHeader (64B cache-aligned)
│   │   ├── kind.rs           # EventKind (private u16)
│   │   ├── hash.rs           # HashChain + compute_hash() + verify_chain() (NO trait)
│   │   └── sourcing.rs       # EventSourced<P> + Reactive<P>
│   │
│   ├── guard/
│   │   ├── mod.rs            # Gate<Ctx> trait, GateSet<Ctx>
│   │   ├── denial.rs         # Denial struct
│   │   └── receipt.rs        # Receipt<T> (sealed, consumed once)
│   │
│   ├── pipeline/
│   │   ├── mod.rs            # Pipeline<Ctx>, Proposal<T>, Committed<T>
│   │   └── bypass.rs         # BypassReason trait, BypassReceipt<T>
│   │
│   ├── store/
│   │   ├── mod.rs            # Store, StoreConfig, StoreError, AppendReceipt, StoredEvent
│   │   ├── segment.rs        # SegmentHeader, frame_encode/decode, FramePayload<P>
│   │   ├── writer.rs         # WriterHandle, WriterCommand, SubscriberList, 10-step commit
│   │   ├── reader.rs         # Reader (LRU FD cache, pread, CRC32 verify)
│   │   ├── index.rs          # StoreIndex, IndexEntry, ClockKey, DiskPos
│   │   ├── projection.rs     # ProjectionCache trait, NoCache, RedbCache, Freshness
│   │   ├── cursor.rs         # Cursor (pull-based, guaranteed delivery)
│   │   └── subscription.rs   # Subscription (push-based, per-subscriber flume channels)
│   │
│   ├── typestate/
│   │   ├── mod.rs            # define_state_machine!, define_typestate!
│   │   └── transition.rs     # Transition<From, To, P>
│   │
│   └── id/
│       └── mod.rs            # EntityIdType trait (Layer 0) + define_entity_id! macro (Layer 1+)
│
├── tests/
│   ├── monad_laws.rs         # proptest: left/right identity, associativity, Batch distribution
│   ├── hash_chain.rs         # proptest: chain verification, tamper detection, genesis
│   ├── store_integration.rs  # tempdir: append/get/query, rotation, cold start, concurrent r/w
│   ├── gate_pipeline.rs      # registration order, fail-fast, receipt TOCTOU, consumed once
│   ├── typestate_safety.rs   # trybuild: compile-fail for invalid transitions + forged receipts
│   ├── wire_format.rs        # golden file comparison for MessagePack serialization
│   └── self_benchmark.rs     # gate that validates cold start < 200ms (library tests itself)
│
└── benches/
    ├── write_throughput.rs   # criterion: events/sec for 1K/10K/100K appends
    ├── cold_start.rs         # criterion: index rebuild for 1K/10K/100K/1M events
    └── projection_latency.rs # criterion: cache hit vs miss for EventSourced projection
```

---

## Cargo.toml

```toml
[package]
name = "free-batteries"
version = "0.1.0"
edition = "2021"
rust-version = "1.75"
license = "MIT OR Apache-2.0"
description = "Event-sourced state machines over coordinate spaces"

[features]
default = ["serde", "blake3"]
serde = ["dep:serde", "dep:serde_json", "uuid/serde"]
blake3 = ["dep:blake3"]
redb = ["dep:redb"]
lmdb = ["dep:heed"]

[dependencies]
uuid = { version = "1", features = ["v7"] }
serde = { version = "1", features = ["derive"], optional = true }
serde_json = { version = "1", optional = true }
blake3 = { version = "1", optional = true }
flume = "0.11"
crc32fast = "1"
rmp-serde = "1"
dashmap = "6"
parking_lot = "0.12"
tracing = "0.1"
redb = { version = "2", optional = true }
heed = { version = "0.20", optional = true }
# NO TOKIO. Invariant 1.

[dev-dependencies]
proptest = "1"
criterion = { version = "0.5", features = ["html_reports"] }
tempfile = "3"
trybuild = "1"
serde_json = "1"
tokio = { version = "1", features = ["rt", "macros"] }

[lints.clippy]
dbg_macro = "deny"
todo = "deny"
unimplemented = "deny"
unwrap_used = "deny"
panic = "deny"
print_stdout = "deny"
print_stderr = "deny"
large_enum_variant = "warn"
clone_on_ref_ptr = "warn"
needless_pass_by_value = "warn"
module_name_repetitions = "allow"
must_use_candidate = "allow"
missing_errors_doc = "allow"

[[bench]]
name = "write_throughput"
harness = false

[[bench]]
name = "cold_start"
harness = false

[[bench]]
name = "projection_latency"
harness = false
```

---

## PER-FILE PRDs

Every PRD below is the FINAL version. No overrides exist. Implement exactly what's described.

---

### `src/lib.rs`

```
Crate root. Module declarations + getting-started guide in doc comments.

Doc comment structure:
  Paragraph 1: "free-batteries is a library for building event-sourced
    state machines over user-defined coordinate spaces."
  Paragraph 2: Four concepts — Coordinate (where), Outcome (what happened),
    Gate (who decides), Store (the runtime).
  Paragraph 3: 12-line hello world (pure sync, fn main, no tokio).
  Paragraph 4: Reading order: coordinate → outcome → event → guard →
    pipeline → store → typestate.

Module declarations in READING ORDER (not alphabetical):
  pub mod coordinate;
  pub mod outcome;
  pub mod event;
  pub mod guard;
  pub mod pipeline;
  pub mod store;
  pub mod typestate;
  pub mod id;
  pub mod prelude;

Each module's doc comment teaches the concept it implements:
  P1: What this module is (one sentence)
  P2: The concept (2-3 sentences, no jargon)
  P3: Minimum example (3-5 lines)
  P4: "Next: read [next module] to learn [next concept]"
```

---

### `src/prelude.rs`

```
15 types for 90% of usage. `use free_batteries::prelude::*`

Re-exports:
  Coordinate, Region, EventKind, DagPosition
  Outcome, OutcomeError, ErrorKind
  Event, EventHeader, HashChain, StoredEvent
  Gate, GateSet, Denial, Receipt
  Proposal, Committed
  Store
  EventSourced
```

---

### `src/coordinate/mod.rs`

```
Coordinate struct + Region struct + CoordinateError + EventKindFilter.

pub struct Coordinate {
    entity: Arc<str>,   // WHO — stream key, hash chain anchor
    scope: Arc<str>,    // WHERE — isolation boundary
}
Coordinate::new(entity: impl AsRef<str>, scope: impl AsRef<str>) -> Result<Self, CoordinateError>
  Validates both non-empty.
entity() -> &str, scope() -> &str
entity_arc() -> Arc<str>, scope_arc() -> Arc<str>  (pub(crate))
Display: "entity@scope"

pub enum CoordinateError { EmptyEntity, EmptyScope }
  impl Display, Error. Coordinate does NOT depend on StoreError.
  StoreError has From<CoordinateError> in the store layer.

pub struct Region {
    pub entity_prefix: Option<Arc<str>>,
    pub scope: Option<Arc<str>>,
    pub fact: Option<EventKindFilter>,
    pub sequence_range: Option<(u32, u32)>,  // per-entity sequence, NOT wall-clock
}

pub enum EventKindFilter {
    Exact(EventKind),
    Category(u8),    // matches any EventKind in this 4-bit category
    Any,
}

Region builder (method chaining):
  Region::all()
  Region::entity("player:alice")
  Region::scope("room:dungeon")
  Region::coordinate(&coord)
  .scope("x").fact(EventKindFilter::Exact(k)).fact_category(0xF)

Region replaces SubscriptionPattern. It is the ONE predicate type for:
  Applied to history  = query
  Applied to future   = subscription (push)
  Applied to cursor   = consumption (pull, guaranteed)
  Applied to chain    = traversal (walk_ancestors is the exception — see store)

Region::matches(&self, notif: &Notification) -> bool
  Used by Subscription to filter incoming events.
```

---

### `src/coordinate/position.rs`

```
DagPosition. Graph position: depth + lane + sequence.

#[repr(C)]
pub struct DagPosition { pub depth: u32, pub lane: u32, pub sequence: u32 }

const fn: new, root, child, fork, is_root, is_ancestor_of
Display: "depth:lane:sequence"
PartialOrd for causal ordering.

v1: depth=0, lane=0 always. Sequence is per-entity monotonic counter.
Batched events get sequential positions (N, N+1, N+2...) on lane 0.
Lane/depth vocabulary is for future distributed fan-out/fan-in.
```

---

### `src/outcome/mod.rs`

```
Outcome<T>. The core algebraic type. Named "Outcome" not "Effect" to
eliminate Effect/Event confusion. The algebra is identical — Outcome IS the
effect functor at different commitment phases.

pub enum Outcome<T> {
    Ok(T),
    Err(OutcomeError),
    Retry { after_ms: u64, attempt: u32, max_attempts: u32, reason: String },
    Pending { condition: WaitCondition, resume_token: u128 },
    Cancelled { reason: String },
    Batch(Vec<Outcome<T>>),
}

6 variants. Compensate was folded into OutcomeError.compensation.
Join is join_all/join_any free functions in combine.rs.

Combinators (all distribute over Batch via F: Clone bound):
  map, and_then, map_err, or_else, flatten, inspect, inspect_err,
  and_then_if, into_result, unwrap_or, unwrap_or_else

Predicates: is_ok, is_err, is_retry, is_pending, is_cancelled, is_batch, is_terminal
Construction: ok, err, cancelled, retry, pending

The and_then monad fix:
  pub fn and_then<U, F: FnOnce(T) -> Outcome<U> + Clone>(self, f: F) -> Outcome<U>
  Distributes over Batch (recurses into each element).

Serde: #[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
  Adjacent tagging: #[serde(tag = "type", content = "data")]
  Durations as u64 millis.
```

---

### `src/outcome/error.rs`

```
OutcomeError + ErrorKind.

pub struct OutcomeError {
    pub kind: ErrorKind,
    pub message: String,
    pub compensation: Option<CompensationAction>,
    pub retryable: bool,
}
impl Display, Error, Clone, PartialEq.

pub enum ErrorKind {
    NotFound, Conflict, Validation, PolicyRejection,
    StorageError, Timeout, Serialization, Internal,
    Custom(u16),
}
ErrorKind::is_retryable(), is_domain(), is_operational()

Products extend via Custom(u16) — same category:type encoding as EventKind.
```

---

### `src/outcome/combine.rs`

```
pub fn zip<A, B>(a: Outcome<A>, b: Outcome<B>) -> Outcome<(A, B)>
pub fn join_all<T>(batch: Vec<Outcome<T>>) -> Outcome<Vec<T>>
pub fn join_any<T>(batch: Vec<Outcome<T>>) -> Outcome<T>
```

---

### `src/outcome/wait.rs`

```
pub enum WaitCondition {
    Timeout { resume_at_ms: u64 },
    Event { event_id: u128 },
    All(Vec<WaitCondition>),
    Any(Vec<WaitCondition>),
    Custom { tag: u16, data: Vec<u8> },
}

pub enum CompensationAction {
    Rollback { event_ids: Vec<u128> },
    Notify { target_id: u128, message: String },
    Release { resource_ids: Vec<u128> },
    Custom { action_type: String, data: Vec<u8> },
}
```

---

### `src/event/mod.rs`

```
Event<P> + StoredEvent<P>.

pub struct Event<P> {
    pub header: EventHeader,
    pub payload: P,
    pub hash_chain: Option<HashChain>,
}
Event::new, with_hash_chain, event_id, event_kind, position, map_payload, is_genesis

pub struct StoredEvent<P> {
    pub coordinate: Coordinate,
    pub event: Event<P>,
}
This is what store.get() returns and what segments persist.
The coordinate is part of the stored fact, not separate metadata.

store.get() returns StoredEvent<serde_json::Value> because segments are
schema-free MessagePack. Round-trip: MyStruct → msgpack → serde_json::Value.
Use project<T: EventSourced>() for typed reconstruction.
```

---

### `src/event/header.rs`

```
#[repr(C, align(64))]
pub struct EventHeader {
    pub event_id: u128,
    pub correlation_id: u128,
    pub timestamp_us: i64,
    pub position: DagPosition,
    pub payload_size: u32,
    pub event_kind: EventKind,
    pub flags: u8,
}

THE STORE GENERATES THIS. Users never call EventHeader::new directly.
store.append(coord, kind, payload) fills in event_id (UUIDv7), timestamp,
position (from index), payload_size (from serialization). Constructor is
pub for testing and advanced use only.

  EventHeader::new(event_id, correlation_id, timestamp_us, position, payload_size, event_kind)
  with_flags(u8) -> Self            // builder
  requires_ack() -> bool            // flag bit 0
  is_transactional() -> bool        // flag bit 1
  is_replay() -> bool               // flag bit 3
  age_us(now_us: i64) -> u64        // convenience: now_us - timestamp_us
```

---

### `src/event/kind.rs`

```
pub struct EventKind(u16);  // PRIVATE inner field

EventKind::custom(category: u8, type_id: u16) -> Self
category() -> u8, type_id() -> u16, is_system(), is_effect()

Library constants ONLY:
  DATA=0x0000, SYSTEM_INIT=0x0001, SYSTEM_SHUTDOWN=0x0002,
  SYSTEM_HEARTBEAT=0x0003, SYSTEM_CONFIG_CHANGE=0x0004,
  SYSTEM_CHECKPOINT=0x0005,
  EFFECT_ERROR=0xD001, EFFECT_RETRY=0xD002, EFFECT_ACK=0xD004,
  EFFECT_BACKPRESSURE=0xD005, EFFECT_CANCEL=0xD006, EFFECT_CONFLICT=0xD007

Products: pub const PLAYER_MOVED: EventKind = EventKind::custom(0xF, 1);
```

---

### `src/event/hash.rs`

```
NO TRAIT. NO ENUM. Blake3 only (Invariant 5).

pub struct HashChain {
    pub prev_hash: [u8; 32],
    pub event_hash: [u8; 32],
}
Default: all zeros (genesis convention).

#[cfg(feature = "blake3")]
pub fn compute_hash(content_bytes: &[u8]) -> [u8; 32]

#[cfg(feature = "blake3")]
pub fn verify_chain(content_bytes: &[u8], chain: &HashChain, expected_prev: &[u8; 32]) -> bool

When blake3 feature is off, Committed.hash is [0u8; 32] (genesis convention).
~30 LOC total.
```

---

### `src/event/sourcing.rs`

```
EventSourced<P> + Reactive<P>.

pub trait EventSourced<P>: Sized {
    fn from_events(events: &[Event<P>]) -> Option<Self>;
    fn apply_event(&mut self, event: &Event<P>);
    fn relevant_event_kinds() -> &'static [EventKind];
}
P is generic. NO serde_json dependency in the trait.
The store uses EventSourced<serde_json::Value> via its serde feature.

pub trait Reactive<P> {
    fn react(&self, event: &Event<P>) -> Vec<(Coordinate, EventKind, P)>;
}
Forward-looking counterpart to EventSourced (backward-looking fold).
See event → maybe emit derived events. ~15 LOC. Same file.
Products compose: subscribe + react + append (7 lines of glue).
```

---

### `src/guard/mod.rs`

```
pub trait Gate<Ctx>: Send + Sync {
    fn name(&self) -> &'static str;
    fn evaluate(&self, ctx: &Ctx) -> Result<(), Denial>;
    fn description(&self) -> &'static str { "" }
}
Gates are PREDICATES, not transformers. No I/O, no mutation, pure.
Ctx is product-defined. Library is generic over it.

pub struct GateSet<Ctx> { gates: Vec<Box<dyn Gate<Ctx>>> }
  push(gate), evaluate(ctx, proposal) -> Result<Receipt<T>, Denial>,
  evaluate_all (no fail-fast, for observability), len, is_empty, names
```

---

### `src/guard/denial.rs`

```
pub struct Denial {
    pub gate: &'static str,
    pub code: String,
    pub message: String,
    pub context: Vec<(String, String)>,
}
Denial::new(gate, message), with_code, with_context
Display: "[gate] message"
Separate from OutcomeError. Library does NOT auto-store denials.
Products decide whether to persist denials as events.
Serde: feature-gated.
```

---

### `src/guard/receipt.rs`

```
pub struct Receipt<T> { _seal: seal::Token, gates_passed: Vec<&'static str>, payload: T }

NOT Clone. NOT Copy. NOT Serialize. Consumed exactly once.

mod seal { pub(crate) struct Token; }  // prevents external construction

Receipt::payload() -> &T
Receipt::gates_passed() -> &[&'static str]
Receipt::into_parts() -> (T, Vec<&'static str>)   // consuming extraction

TOCTOU fix: payload is INSIDE the receipt. Cannot mutate after gate evaluation.
Only constructible via GateSet::evaluate().
```

---

### `src/pipeline/mod.rs`

```
pub struct Proposal<T>(pub T);
  Proposal::new, payload, map

pub struct Committed<T> {
    pub payload: T,
    pub event_id: u128,
    pub sequence: u64,
    pub hash: [u8; 32],   // blake3, or [0u8;32] if feature off
}

pub struct Pipeline<Ctx> { gates: GateSet<Ctx> }
  Pipeline::new(gates)
  evaluate(ctx, proposal) -> Result<Receipt<T>, Denial>
  commit(receipt, commit_fn) -> Result<Committed<T>, E>

Library owns 2 stages: evaluate + commit.
Products wrap with assembly (before) and receipt generation (after).
```

---

### `src/pipeline/bypass.rs`

```
pub trait BypassReason: Send + Sync {
    fn name(&self) -> &'static str;
    fn justification(&self) -> &'static str;
}

pub struct BypassReceipt<T> {
    pub payload: T,
    pub reason: &'static str,
    pub justification: &'static str,
}

Pipeline::bypass(proposal, reason) -> BypassReceipt<T>
Audit trails show "bypassed: {reason}" with empty gate list.
```

---

### `src/store/mod.rs`

```
pub struct Store { index, reader, cache, writer, config }

Store::open(config: StoreConfig) -> Result<Self, StoreError>
Store::open_default() -> Result<Self, StoreError>  // ./free-batteries-data/

ALL METHODS ARE SYNC (Invariant 2):

WRITE:
  append(&self, coord: &Coordinate, kind: EventKind, payload: &impl Serialize)
    -> Result<AppendReceipt, StoreError>
    3 params. Store generates event_id, timestamp, position, hash chain.
  append_with_options(..., opts: AppendOptions) — CAS, idempotency
  apply_transition(coord, transition) — extracts kind+payload, delegates to append

READ:
  get(event_id: u128) -> Result<StoredEvent<serde_json::Value>, StoreError>
  query(region: &Region) -> Vec<IndexEntry>
  walk_ancestors(event_id: u128, limit: usize) -> Vec<StoredEvent<Value>>
    (special case — NOT Region-based; chain traversal is point-to-chain)

PROJECT:
  project<T: EventSourced<serde_json::Value>>(entity: &str, freshness: Freshness)
    -> Result<Option<T>, StoreError>

SUBSCRIBE:
  subscribe(region: &Region) -> Subscription     // push, per-subscriber flume channel
  cursor(region: &Region) -> Cursor              // pull, guaranteed delivery

CONVENIENCE (sugar over Region):
  stream(entity) = query(&Region::entity(entity))
  by_scope(scope) = query(&Region::scope(scope))
  by_fact(kind) = query(&Region::all().fact(EventKindFilter::Exact(kind)))

LIFECYCLE:
  sync(), snapshot(dest), compact(), close(self)

DIAGNOSTICS:
  stats() -> StoreStats, diagnostics() -> StoreDiagnostics

Async callers: use tokio::task::spawn_blocking(|| store.append(...)).await
Or use flume's async API on the channels directly.

Types owned by this module:
  Store, StoreConfig, StoreError (with From<CoordinateError>),
  StoreStats, StoreDiagnostics, StoredEvent<P>, AppendReceipt, AppendOptions
```

---

### `src/store/segment.rs`

```
Magic: b"FBAT", Header: 32 bytes, Frame: [len:u32 BE][crc32:u32 BE][msgpack]

SegmentHeader { version: u16, flags: u16, created_ns: i64, segment_id: u64 }
frame_encode(data) -> Vec<u8>
frame_decode(buf) -> Result<(&[u8], usize), StoreError>
FramePayload<P> { event: Event<P>, entity: String, scope: String }

Typestate: Segment<Active> (writable) vs Segment<Sealed> (immutable).
Rotation: seal + create new when segment exceeds config.segment_max_bytes.
Types: SegmentHeader, FramePayload<P>, CompactionResult
```

---

### `src/store/writer.rs`

```
Background OS thread ("free-batteries-writer"). Sync-first. flume channels.

WriterCommand { Append{entity,scope,event,respond}, Sync{respond}, Shutdown{respond} }
  All respond channels: flume::Sender (sync send from writer, async recv from caller)

WriterHandle { tx: flume::Sender<WriterCommand>, subscribers: Arc<SubscriberList>, thread }

struct SubscriberList {
    senders: parking_lot::Mutex<Vec<flume::Sender<Notification>>>,
}
  broadcast(notif): iterate, send, retain(ok). NO tokio::broadcast.
  subscribe(capacity) -> flume::Receiver<Notification>

Notification { event_id: u128, coord: Coordinate, kind: EventKind, sequence: u64 }

The 10-step commit protocol (handle_append):
  1. Acquire per-entity lock (DashMap<Arc<str>, Arc<Mutex<()>>>)
  2. Get prev_hash from index (or [0u8;32] for genesis)
  3. Compute sequence (latest.clock + 1, or 0)
  4. Set event header position
  5. Compute blake3 hash, set hash chain (or skip if feature off)
  6. Serialize to MessagePack + CRC32 frame
  7. Check segment rotation
  8. Write frame to segment file
  9. Update index
  10. Broadcast notification to subscribers

Backpressure: bounded channel (default 4096). Callers block when full.
Entity locks: grow without pruning (acceptable <100K entities).
Crash recovery: if writer panics, WriterHandle detects (flume send fails).
  Store restarts writer once. Second panic → StoreError::WriterCrashed.

// NOTE: CompensationAction exists on OutcomeError but the writer
// ignores it. Compensation handling is deferred — products implement it.

Tracing spans:
  warn!  — CRC mismatch, writer panic, segment corruption
  info!  — segment rotation, cold start complete, cache miss
  debug! — append committed, entity lock acquired, fsync
  trace! — frame written
```

---

### `src/store/reader.rs`

```
Reader with LRU file descriptor cache.
Reader::new(data_dir, fd_budget)
read_entry(disk_pos) -> Result<StoredEvent<serde_json::Value>, StoreError>
scan_segment(path) -> Result<Vec<ScannedEntry>, StoreError>
CRC32 verified on every read.
```

---

### `src/store/index.rs`

```
2D primary index + auxiliaries (NOT "4D" — fact and clock are event metadata).

pub(crate) struct StoreIndex {
    streams: DashMap<Arc<str>, BTreeMap<ClockKey, IndexEntry>>,     // primary
    scope_entities: DashMap<Arc<str>, HashSet<Arc<str>>>,           // scope dim
    by_fact: DashMap<EventKind, BTreeMap<ClockKey, IndexEntry>>,    // fact dim
    by_id: DashMap<u128, IndexEntry>,                               // point lookup
    latest: DashMap<Arc<str>, IndexEntry>,                          // chain head
    global_sequence: AtomicU64,  // monotonic counter (foundation for:
                                 //   1. cursors — track position
                                 //   2. checkpoints — record sequence at snapshot
                                 //   3. exactly-once — consumers track high-water mark)
    len: AtomicUsize,
}

pub struct IndexEntry {
    pub event_id: u128,
    pub coord: Coordinate,
    pub kind: EventKind,
    pub clock: u32,
    pub hash_chain: HashChain,
    pub disk_pos: DiskPos,
    pub global_sequence: u64,
}

pub struct DiskPos { pub segment_id: u64, pub offset: u64, pub length: u32 }
pub struct ClockKey { pub clock: u32, pub uuid: u128 }

Memory: ~200-300 bytes per IndexEntry. 10M events ≈ 2-3GB RAM. No eviction.
```

---

### `src/store/projection.rs`

```
pub trait ProjectionCache: Send + Sync + 'static {
    fn get(&self, key: &[u8]) -> Result<Option<(Vec<u8>, CacheMeta)>, StoreError>;
    fn put(&self, key: &[u8], value: &[u8], meta: CacheMeta) -> Result<(), StoreError>;
    fn delete_prefix(&self, prefix: &[u8]) -> Result<u64, StoreError>;
    fn sync(&self) -> Result<(), StoreError>;
}

pub struct CacheMeta { pub watermark: u64, pub cached_at_us: i64 }
pub enum Freshness { Consistent, BestEffort { max_stale_ms: u64 } }
pub struct NoCache;   // default: every read replays from segments

#[cfg(feature = "redb")] pub struct RedbCache { ... }
#[cfg(feature = "lmdb")] pub struct LmdbCache { ... }
```

---

### `src/store/cursor.rs`

```
Pull-based event consumption with guaranteed delivery.

pub struct Cursor { region: Region, position: u64, store: Arc<StoreIndex> }
  poll() -> Option<IndexEntry>         // next matching event after position
  poll_batch(max: usize) -> Vec<IndexEntry>

Reads from index, not channels. Cannot lose events.
```

---

### `src/store/subscription.rs`

```
Push-based per-subscriber flume channels. NO tokio::broadcast.

pub struct Subscription { rx: flume::Receiver<Notification>, region: Region }
  recv() -> Option<Notification>              // sync: blocks
  receiver() -> &flume::Receiver<Notification>  // for async: rx.recv_async().await

Lossy: if subscriber is slow, bounded channel fills. Writer's retain()
prunes dropped senders. For guaranteed delivery, use Cursor instead.

ASYNC NOTE: For async event consumption, use receiver().recv_async().await
directly on the flume channel. spawn_blocking is for Store read/write
methods only (append, get, query, project). These are two different async
patterns for two different things — don't conflate them.
```

---

### `src/typestate/mod.rs`

```
macro_rules! define_state_machine! { ... }
  Generates: sealed marker trait + zero-sized state structs.

macro_rules! define_typestate! { ... }
  Generates: PhantomData wrapper with data(), into_data(), new().

99 LOC of macros. Zero deps. Zero runtime code.
```

---

### `src/typestate/transition.rs`

```
pub struct Transition<From, To, P> {
    kind: EventKind,
    payload: P,
    _from: PhantomData<From>,
    _to: PhantomData<To>,
}
Transition::new(kind, payload), kind(), payload(), into_payload()

Usage:
  impl Lock<Acquired> {
      pub fn release(self) -> Transition<Acquired, Released, ()> {
          Transition::new(LOCK_RELEASED, ())
      }
  }
  store.apply_transition(&coord, lock.release())?;

Store extracts EventKind + payload, builds Event, appends.
Compiler ensures you can only call release() on Lock<Acquired>.
EventSourced replays by matching on EventKind.
Forward (Transition) and backward (EventSourced) share EventKind as common language.
```

---

### `src/id/mod.rs`

```
Layer 0 (trait, no uuid dep):
  pub trait EntityIdType:
      Copy + Clone + Eq + Hash + Debug + Display + FromStr + Send + Sync + 'static
  {
      const ENTITY_NAME: &'static str;
      fn new(id: u128) -> Self;
      fn as_u128(&self) -> u128;
      fn now_v7() -> Self;
      fn nil() -> Self;
  }

Layer 1+ (macro, uses uuid):
  macro_rules! define_entity_id! { ($name:ident, $entity:literal) => { ... } }

Library defines ONE id: define_entity_id!(EventId, "event");
Products: define_entity_id!(PlayerId, "player");

All IDs use u128 internally. No Uuid type in public API.
uuid crate used only inside the macro's now_v7() implementation.
```

---

## CONTROL FLOW

### Write Path

```
User ── store.append(coord, kind, payload) ──> Store
  Store builds Event (header, payload, no hash yet)
  Store sends WriterCommand::Append via flume::send() [blocks if full]
  ──> Writer thread:
      1. entity lock
      2. prev_hash from index
      3. sequence = latest+1
      4. set position
      5. blake3 hash + hash chain
      6. msgpack + crc32 frame
      7. rotation check
      8. write frame to segment
      9. update index
      10. broadcast to subscribers (Vec<flume::Sender>)
  <── flume::recv() response
User gets AppendReceipt { event_id, sequence, disk_pos }
```

### Pipeline Flow

```
Product assembles (Ctx, Proposal<T>) via I/O
  ──> pipeline.evaluate(ctx, proposal)
      GateSet runs each Gate<Ctx>::evaluate(&ctx)
      All pass → Receipt<T> (sealed, wraps payload)
      Any deny → Err(Denial)
  ──> pipeline.commit(receipt, commit_fn)
      receipt.into_parts() → (payload, gate_names)
      commit_fn(payload) → store.append(...)
  ──> Committed<T> { payload, event_id, sequence, hash }
```

### Projection Flow

```
store.project::<Player>("player:1", Consistent)
  ──> index.latest("player:1") → watermark
  ──> cache.get(key) → HIT? return cached. MISS? continue:
  ──> stream all events for "player:1"
  ──> reader.read_entry(disk_pos) for each → Event<Value>
  ──> Player::from_events(&events) → Option<Player>
  ──> cache.put(key, serialized, meta)
  ──> return Player
```

### Subscription vs Cursor

```
PUSH (Subscription — lossy, real-time):
  Writer ──broadcast──> per-subscriber flume channel ──Region filter──> Consumer
  Dropped subscribers pruned on next broadcast (send returns Err)

PULL (Cursor — guaranteed, batch):
  Consumer ──poll()──> Cursor reads index from global_sequence position
  Never loses events. Catches up on next poll.
```

---

## DEVOPS

### rust-toolchain.toml
```toml
[toolchain]
channel = "stable"
components = ["rustfmt", "clippy", "rust-src"]
```

### clippy.toml
```toml
msrv = "1.75"
cognitive-complexity-threshold = 25
too-many-arguments-threshold = 7
```

### .config/nextest.toml
```toml
[profile.default]
retries = 0
slow-timeout = { period = "30s", terminate-after = 2 }
fail-fast = true

[profile.ci]
retries = 2
fail-fast = false
test-threads = "num-cpus"

[profile.default.junit]
path = "target/nextest/default/junit.xml"
```

### .cargo/config.toml
```toml
[profile.dev]
opt-level = 0
debug = true
incremental = true

[profile.dev.package."*"]
opt-level = 2

[profile.release]
opt-level = 3
lto = "thin"
codegen-units = 16
strip = "symbols"

[profile.test]
opt-level = 1

[profile.bench]
inherits = "release"
debug = true
```

### .github/workflows/ci.yml
```yaml
name: CI
on: [push, pull_request]
env:
  CARGO_TERM_COLOR: always
  RUSTFLAGS: "-D warnings"
  PROPTEST_CASES: 1000

jobs:
  check:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - uses: Swatinem/rust-cache@v2
      - run: cargo check --all-features
      - run: cargo check --no-default-features
      - run: cargo check --features serde
      - run: cargo check --features blake3
      - run: cargo check --features redb

  test:
    runs-on: ubuntu-latest
    needs: check
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - uses: taiki-e/install-action@nextest
      - uses: Swatinem/rust-cache@v2
      - run: cargo nextest run --profile ci --all-features
      - run: cargo test --doc --all-features

  clippy:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with: { components: clippy }
      - uses: Swatinem/rust-cache@v2
      - run: cargo clippy --all-features -- -D warnings

  fmt:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with: { components: rustfmt }
      - run: cargo fmt --check

  msrv:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@1.75.0
      - run: cargo check --all-features

  semver:
    runs-on: ubuntu-latest
    if: github.event_name == 'pull_request'
    steps:
      - uses: actions/checkout@v4
      - uses: obi1kenobi/cargo-semver-checks-action@v2

  bench-compile:
    runs-on: ubuntu-latest
    needs: check
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - uses: Swatinem/rust-cache@v2
      - run: cargo bench --no-run --all-features
```

### justfile
```makefile
default:
    just --list

check:
    cargo check --all-features

test:
    cargo nextest run --all-features
    cargo test --doc --all-features

clip:
    cargo clippy --all-features -- -D warnings

fmt:
    cargo fmt

ci: fmt clip test
    cargo bench --no-run --all-features
    cargo check --no-default-features

bench:
    cargo bench --all-features

doc:
    cargo doc --all-features --no-deps --open
```

### Version Policy
```
MSRV: 1.75. Edition: 2021. Semver: 0.x (breaking changes expected).
Use LATEST version of every dep UNLESS it creates duplicates.
Check: cargo tree --duplicates. If duplicates: pin to older.
```

---

## BUILD ORDER

```
1.  cargo init free-batteries --lib
2.  Create all config files (.cargo, clippy.toml, rust-toolchain.toml,
    nextest.toml, ci.yml, justfile, .gitignore, licenses, changelog)
3.  Write Cargo.toml
4.  Implement Layer 0 (coordinate, outcome core, guard trait, typestate, id trait)
    → cargo check --no-default-features MUST PASS
5.  Implement Layer 1 (serde derives on Layer 0 types)
    → cargo check --features serde MUST PASS
6.  Implement Layer 2 (EventId, EventHeader, Event<P>, hash functions)
    → cargo check --features "serde blake3" MUST PASS
7.  Implement Layer 3 (Store: writer, reader, segment, index, projection, cursor, subscription)
    → cargo check --all-features MUST PASS
8.  Write tests (monad_laws, hash_chain, store_integration, gate_pipeline,
    typestate_safety, wire_format, self_benchmark)
    → cargo nextest run MUST PASS
9.  Write benches → cargo bench --no-run MUST COMPILE
10. Write hello world, verify it runs (fn main, no tokio)
11. cargo clippy --all-features -- -D warnings → ZERO WARNINGS
12. cargo fmt --check → PASSES
13. cargo doc --all-features --no-deps → CLEAN
```

---

## HELLO WORLD (pure sync, no tokio)

```rust
use free_batteries::prelude::*;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let store = Store::open_default()?;
    let coord = Coordinate::new("player:alice", "room:dungeon")?;
    let kind = EventKind::custom(0xF, 1);

    let receipt = store.append(&coord, kind, &serde_json::json!({"x": 10, "y": 20}))?;
    println!("Stored event {} at position {}", receipt.event_id, receipt.sequence);

    for entry in store.stream("player:alice") {
        let stored = store.get(entry.event_id)?;
        println!("{}: {:?}", stored.event.event_kind(), stored.event.payload);
    }
    Ok(())
}
```

---

## DESIGN BOUNDARIES

No v1/v2. No roadmap. The library does X. If pain demands Y, you add Y.
There's no version boundary — just responding to reality.

```
DECIDED (the design — not compromises, not simplifications, the right call):

  Outcome<T> with 1 type param
  Gates are predicates (enrichment goes in assembly, before gates)
  Coordinate is a struct (Arc<str> entity + scope)
  Store is a concrete struct (no storage trait)
  Per-entity linear chains (not a real DAG — DAG vocabulary is scaffolding)
  Lane = 0 always (fan-out is future scaffolding)
  NoCache is the default projection backend
  macro_rules! for typestate (not proc macro)
  Denial is separate from OutcomeError (library doesn't auto-store denials)
  Writer restarts once on panic (not a full supervisor)
  blake3 only (no enum, no trait, one function)
  flume for all channels (no tokio::broadcast)
  Store API is sync (bisync is a channel property)

SHIPPED (enforcement via tests, not types — equally valid, better ergonomics):

  Functor laws — tested via proptest (left/right identity, associativity,
    Batch distribution). The algebra IS the implementation. The 4-param
    Outcome<C,K,P,S> would encode these at compile time. The test suite
    enforces them at test time. Same guarantees, less type noise.

  Coordinate querying — Region ships as the coordinate-space query language.
    Region::entity().scope().fact_category() IS the coordinate algebra.
    A Coordinate trait would let users define custom dimensions. That arrives
    when someone has a coordinate space that isn't two strings.

  Compensation model — CompensationAction ships as data on OutcomeError.
    The writer persists it as part of error events. Products implement the
    handler. Library provides the data model, product provides the execution.

  Checkpoint foundation — global_sequence on every IndexEntry. SYSTEM_CHECKPOINT
    reserved. Checkpoint payload = serialized StoreIndex. Cold start scans
    segments. The ONLY thing that arrives later is ~50 LOC of emission +
    accelerated cold start. Foundation is complete.

ARRIVES WITH CONCRETE PAIN (not deferred — literally doesn't exist because
nobody has hit the wall that demands it):

  EventDag storage trait — when a second backend exists, extract the trait
    from the concrete Store. Until then, Invariant 4.
  derive(EventSourced) proc macro — when 10+ entities make hand-written
    impls painful. EventSourced<P> being generic is the prerequisite.
  Index memory eviction — when a store exceeds 10M events and 2-3GB RAM
    matters. global_sequence + checkpoints enable partial loading.
  Entity lock pruning — when unique entity count exceeds 100K.
    DashMap entry() API makes pruning safe when the time comes.

```

---

## ESTIMATED LOC

```
coordinate/    ~160  (Coordinate + Region + CoordinateError + EventKindFilter + DagPosition)
outcome/       ~450  (Outcome<T> + OutcomeError + combinators + wait conditions)
event/         ~400  (Event<P> + StoredEvent + EventHeader + EventKind + hash fns + EventSourced + Reactive)
guard/         ~250  (Gate + GateSet + Denial + Receipt)
pipeline/      ~150  (Pipeline + Proposal + Committed + Bypass)
store/        ~1800  (Store + segment + writer + reader + index + projection + cursor + subscription)
typestate/     ~160  (macros + Transition<From,To,P>)
id/             ~80  (EntityIdType + define_entity_id! + EventId)
lib+prelude     ~60

CODE:  ~3,510
TESTS: ~1,500
TOTAL: ~5,010
```

---

## VERIFICATION TRACE (reviewed 2026-03-20)

All drift corrections verified against this document:

```
[✓] tokio::broadcast → Vec<flume::Sender>
    writer.rs has SubscriberList with Mutex<Vec<flume::Sender<Notification>>>.
    subscription.rs says "NO tokio::broadcast."
    Cargo.toml has "# NO TOKIO. Invariant 1." tokio only in dev-deps.

[✓] Store API sync
    Every method returns Result, not impl Future.
    store/mod.rs says "ALL METHODS ARE SYNC (Invariant 2)."
    Async callers use spawn_blocking or flume recv_async.

[✓] Watcher → Reactive<P> trait
    ~15 LOC in sourcing.rs next to EventSourced<P>.
    No watcher module. No watcher file.

[✓] Region builder — method chaining
    Region::entity().scope().fact_category()

[✓] blake3 only
    hash.rs: "NO TRAIT. NO ENUM." Two functions + one struct. ~30 LOC.

[✓] EventKindFilter in coordinate/mod.rs
    Region component, Region lives in coordinate. Internally consistent.

[✓] OutcomeError naming
    Consistent throughout. Prelude lists OutcomeError. No stale EffectError.

[✓] StoredEvent<P>
    store.get() returns StoredEvent<serde_json::Value>.
    Round-trip erasure documented.

[✓] CoordinateError
    Coordinate doesn't depend on StoreError.
    From<CoordinateError> lives in store layer.

[✓] Committed<T> hash field
    [u8; 32], always present. [0u8; 32] when blake3 off (genesis convention).

[✓] append signature
    payload: &impl Serialize. Generic.

[✓] Async pattern clarification
    subscription.rs distinguishes recv_async (for subscriptions) from
    spawn_blocking (for Store read/write methods).
```

This document is FINAL. No override layers. No patches. No "THIS SECTION WINS."
An implementing agent reads top to bottom and builds exactly what's described.
