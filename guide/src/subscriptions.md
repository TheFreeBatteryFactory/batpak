# Subscriptions

Use `subscribe_lossy` for bounded push delivery and `cursor_guaranteed` for pull-based replay-style traversal.

## `scan()` is lossy

`SubscriptionOps::scan(initial, f)` folds the lossy notification stream into derived state. It is ideal for dashboards, live counters, and UI projections, but it does **not** upgrade the underlying delivery semantics. If the subscriber is slow, notifications can still be dropped.

Use `cursor_worker(...)` when the fold must be restartable and lossless.
