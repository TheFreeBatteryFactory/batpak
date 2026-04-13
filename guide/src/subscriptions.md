# Subscriptions

Two consumption models: push (lossy) and pull (guaranteed).

## subscribe_lossy — push, bounded, may drop

```rust
use batpak::prelude::*;

let sub = store.subscribe_lossy(&Region::entity("user:alice"));
while let Some(notif) = sub.recv() {
    println!("event {} kind {:?}", notif.event_id, notif.kind);
}
```

Lossy means: if the subscriber is slower than the writer, notifications are dropped silently. The writer never blocks on slow subscribers.

### Composable ops

```rust
let mut ops = store.subscribe_lossy(&Region::entity("user:alice"))
    .ops()
    .filter(|n| n.kind == EventKind::custom(0xF, 1))
    .take(100);

while let Some(notif) = ops.recv() {
    // filtered, limited to 100
}
```

### scan() — live lossy fold

Fold the notification stream into derived state. Good for dashboards, live counters, approximate UI state:

```rust
let mut live = store.subscribe_lossy(&Region::entity("counter:hits"))
    .ops()
    .scan(0u64, |count, _notif| {
        *count += 1;
        Some(*count)
    });

// live.recv() yields the running count after each notification
while let Some(current) = live.recv() {
    println!("count: {current}");
}
```

**Important**: `scan()` does NOT upgrade delivery to guaranteed. If the subscriber falls behind, events are skipped and the fold state will drift. Use `cursor_worker` when the fold must be exact.

## cursor_guaranteed — pull, lossless

```rust
let mut cursor = store.cursor_guaranteed(&Region::all());
while let Some(entry) = cursor.poll() {
    let event = store.get(entry.event_id)?;
    // guaranteed: cursor tracks position, never loses events
}
```

Cursors read from the in-memory index. They cannot lose events because they track position and the index is append-only.

### cursor_worker — supervised background consumer

```rust
use batpak::prelude::*;
use batpak::store::cursor::{CursorWorkerConfig, CursorWorkerAction};
use std::time::Duration;

let handle = store.cursor_worker(
    &Region::entity("order:*"),
    CursorWorkerConfig {
        batch_size: 100,
        idle_sleep: Duration::from_millis(50),
        restart: RestartPolicy::Bounded {
            max_restarts: 3,
            within_ms: 60_000,
        },
    },
    |batch, store| {
        for entry in batch {
            // process entry
        }
        CursorWorkerAction::Continue
    },
)?;

// Worker runs in background thread. On handler panic:
// - restarts from last committed cursor position
// - retries the failed batch
// - stops if restart budget exhausted

handle.stop(); // graceful shutdown
handle.join()?;
```

## When to use which

| Need | Use | Why |
|---|---|---|
| Real-time UI/dashboard | `subscribe_lossy` + `scan()` | Fast, fire-and-forget, OK to miss |
| Exactly-once processing | `cursor_worker` | Guaranteed delivery, restart from checkpoint |
| Ad-hoc replay/query | `cursor_guaranteed` + manual poll | Simple pull loop, no background thread |
| Reactive projection | `watch_projection` | Auto re-projects on new events |
