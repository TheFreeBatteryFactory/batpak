# Append and query

## Single event

```rust
use batpak::prelude::*;
use batpak::store::{Store, StoreConfig};

let store = Store::open(StoreConfig::new("./data"))?;
let coord = Coordinate::new("user:alice", "chat:general")?;
let kind = EventKind::custom(0xF, 1);

let receipt = store.append(&coord, kind, &serde_json::json!({"text": "hello"}))?;
println!("persisted at sequence {}", receipt.sequence);

let event = store.get(receipt.event_id)?;
println!("entity: {}, payload: {}", event.coordinate.entity(), event.event.payload);
```

## Batch append

For atomic bulk insertion, use `Store::append_batch`:

```rust
use batpak::prelude::*;
use batpak::store::{BatchAppendItem, CausationRef};

let items = vec![
    BatchAppendItem::new(
        Coordinate::new("user:alice", "chat:general")?,
        EventKind::custom(1, 1),
        &serde_json::json!({"text": "Hello"}),
        AppendOptions::default(),
        CausationRef::None,
    )?,
    BatchAppendItem::new(
        Coordinate::new("user:bob", "chat:general")?,
        EventKind::custom(1, 1),
        &serde_json::json!({"text": "Hi!"}),
        AppendOptions::default(),
        CausationRef::PriorItem(0), // caused by alice's message
    )?,
];

let receipts = store.append_batch(items)?;
// All events committed atomically or none visible
```

**Key properties:**
- All events in a batch share a single fsync — significantly higher throughput than individual appends
- Atomic visibility: subscribers see all events or none (no partial batches)
- Crash recovery: incomplete batches (missing COMMIT marker) are discarded on restart
- `CausationRef` links events within a batch: `PriorItem(index)` causes event at that index

## Query patterns

```rust
// All events for an entity (exact match, not prefix):
let events = store.stream("user:alice");

// All events matching a region (prefix, scope, kind, or combined):
let events = store.query(&Region::entity("user:"));       // prefix match
let events = store.query(&Region::scope("chat:general")); // scope match
let events = store.by_fact(EventKind::custom(0xF, 1));    // by kind
let events = store.by_scope("chat:general");              // convenience

// Combined:
let events = store.query(
    &Region::scope("chat:general")
        .with_fact(KindFilter::Exact(EventKind::custom(1, 1)))
);
```

All query methods return `Vec<IndexEntry>` from the in-memory index — no disk I/O. To read the full event payload, use `store.get(entry.event_id)`.

## Append options

### Compare-and-swap (CAS)

```rust
let opts = AppendOptions::new().with_cas(expected_sequence);
store.append_with_options(&coord, kind, &payload, opts)?;
// Fails with StoreError::SequenceMismatch if entity's sequence != expected
```

### Idempotency keys

```rust
let opts = AppendOptions::new().with_idempotency(0xDEADBEEF_u128);
store.append_with_options(&coord, kind, &payload, opts)?;
// Second append with same key is silently deduplicated
```

Required when `group_commit_max_batch > 1` (group commit can replay appends after crash).

## EventKind

Events are typed by `EventKind` — a packed u16 with 4-bit category and 12-bit type:

```rust
let msg_sent = EventKind::custom(0x1, 1);    // category 1, type 1
let msg_edited = EventKind::custom(0x1, 2);  // category 1, type 2
let user_joined = EventKind::custom(0x2, 1); // category 2, type 1

// Categories 0x1-0xC, 0xE-0xF are user-defined
// Category 0x0 is system-reserved
// Category 0xD is effect-reserved
```
