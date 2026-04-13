# Control plane

`batpak` still keeps the simple path simple:

```rust
let receipt = store.append(&coord, kind, &payload)?;
```

That blocking API remains the default onboarding surface. Internally it now rides on the richer submission path, so callers that need pipelining or explicit backpressure can opt into it without introducing an async runtime.

## Submit and tickets

Use `submit` when you want to enqueue first and wait later:

```rust
let t1 = store.submit(&coord, kind, &serde_json::json!({"n": 1}))?;
let t2 = store.submit(&coord, kind, &serde_json::json!({"n": 2}))?;

let r1 = t1.wait()?;
let r2 = t2.wait()?;
```

`AppendTicket` and `BatchAppendTicket` expose:

- `wait(self)` for the synchronous path
- `try_check(&self)` for polling
- `receiver(&self)` when a caller wants to bridge into its own `flume`-based coordination

## Soft pressure with `Outcome`

Use `try_submit*` when the caller wants a richer answer than "blocked until done":

```rust
match store.try_submit(&coord, kind, &payload)? {
    batpak::outcome::Outcome::Ok(ticket) => {
        let receipt = ticket.wait()?;
        println!("committed at sequence {}", receipt.sequence);
    }
    batpak::outcome::Outcome::Retry { after_ms, reason, .. } => {
        println!("writer is busy: {reason}; retry after {after_ms}ms");
    }
    batpak::outcome::Outcome::Cancelled { reason } => {
        println!("submission was refused: {reason}");
    }
    other => unreachable!("store try_submit only returns terminal control outcomes in this wave: {other:?}"),
}
```

The default soft-pressure rule triggers once the writer mailbox reaches 75% of capacity. Tune it with `StoreConfig::with_writer_pressure_retry_threshold_pct(...)`.

Use `store.writer_pressure()` for direct telemetry:

- `queue_len`
- `capacity`
- `utilization()`
- `headroom()`
- `is_idle()`

## Outbox batching

`Outbox` stages validated `BatchAppendItem`s on the producer side, then flushes them through the batch path:

```rust
let mut outbox = store.outbox();
outbox.stage(coord.clone(), kind, &serde_json::json!({"n": 1}))?;
outbox.stage_with_options(
    coord.clone(),
    kind,
    &serde_json::json!({"n": 2}),
    batpak::store::AppendOptions::new().with_idempotency(0xAABB),
)?;

let receipts = outbox.flush()?;
assert_eq!(receipts.len(), 2);
```

Use `submit_flush()` when you want a batch ticket instead of blocking immediately.

## Visibility fences

`VisibilityFence` gives you "durable now, visible together later" semantics:

```rust
let fence = store.begin_visibility_fence()?;
let ticket = fence.submit(&coord, kind, &serde_json::json!({"n": 1}))?;

// Event is validated, written, indexed, and durable, but not yet visible.
assert!(ticket.try_check().is_none());

fence.commit()?;
let receipt = ticket.wait()?;
println!("published together at sequence {}", receipt.sequence);
```

Wave-1 rules:

- only one public fence may be active at a time
- normal store writes are rejected while a public fence is active
- dropping or cancelling a fence keeps its writes durable but hidden
- hidden cancelled ranges survive reopen, snapshot copy, and fast-start restore

`fence.outbox()` gives you the same producer-side staging surface inside the fence.

## Read-only mode

Use `Store<ReadOnly>` when you need queries, projections, and cursor-style reads without starting the writer thread:

```rust
let ro = batpak::store::Store::<batpak::store::ReadOnly>::open_read_only(config)?;
let events = ro.by_fact(kind);
```

Read-only open still uses the fast cold-start path:

1. `index.fbati` mmap snapshot if valid
2. checkpoint restore if available
3. segment rebuild fallback

## Live folds vs guaranteed workers

`SubscriptionOps::scan()` folds the lossy subscription stream into derived state:

```rust
let mut live = store
    .subscribe_lossy(&batpak::coordinate::Region::entity("counter"))
    .ops()
    .scan(0u64, |count, _| {
        *count += 1;
        Some(*count)
    });
```

This is great for dashboards and UI-ish live state, but it is still built on the lossy subscription path. If the subscriber falls behind, events may be skipped.

For guaranteed delivery, use `cursor_worker(...)` on the cursor path instead. Cursor workers restart from the last committed cursor position according to `RestartPolicy`.
