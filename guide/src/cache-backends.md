# Cache backends

Use:

- `Store::open_with_redb_cache(...)` with the `redb` feature
- `Store::open_with_lmdb_cache(...)` with the `lmdb` feature

These wrap the lower-level `open_with_cache` API for common setups.
