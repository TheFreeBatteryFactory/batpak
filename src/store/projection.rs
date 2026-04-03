use crate::store::StoreError;
use serde::{Deserialize, Serialize};

/// Describes optional capabilities supported by a cache backend.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct CacheCapabilities {
    /// Whether this backend supports `prefetch()` hints for pre-warming.
    pub supports_prefetch: bool,
}

impl CacheCapabilities {
    /// Return capabilities with no optional features enabled.
    pub const fn none() -> Self {
        Self {
            supports_prefetch: false,
        }
    }

    /// Return capabilities indicating support for prefetch hints.
    pub const fn prefetch_hints() -> Self {
        Self {
            supports_prefetch: true,
        }
    }
}

/// Trait for caching projected state. Three impls: `NoCache` (default), `RedbCache`, `LmdbCache`.
pub trait ProjectionCache: Send + Sync + 'static {
    /// Return the capabilities advertised by this cache backend.
    fn capabilities(&self) -> CacheCapabilities;
    /// Retrieve a cached value and its metadata by key. Returns `None` on a cache miss.
    ///
    /// # Errors
    /// Returns `StoreError::CacheFailed` if the underlying cache backend fails.
    fn get(&self, key: &[u8]) -> Result<Option<(Vec<u8>, CacheMeta)>, StoreError>;
    /// Store a value with associated metadata under the given key.
    ///
    /// # Errors
    /// Returns `StoreError::CacheFailed` if the underlying cache backend fails.
    fn put(&self, key: &[u8], value: &[u8], meta: CacheMeta) -> Result<(), StoreError>;
    /// Delete all entries whose keys start with the given prefix. Returns the number of entries removed.
    ///
    /// # Errors
    /// Returns `StoreError::CacheFailed` if the underlying cache backend fails.
    fn delete_prefix(&self, prefix: &[u8]) -> Result<u64, StoreError>;
    /// Flush any pending writes to durable storage.
    ///
    /// # Errors
    /// Returns `StoreError::CacheFailed` if flushing the cache backend fails.
    fn sync(&self) -> Result<(), StoreError>;

    /// Hint that this key is likely to be requested soon. Implementations may
    /// pre-warm internal caches or pre-compute values. Default: no-op.
    ///
    /// # Errors
    /// Returns [`StoreError::CacheFailed`] if the prefetch operation fails.
    fn prefetch(&self, _key: &[u8], _predicted_meta: CacheMeta) -> Result<(), StoreError> {
        Ok(()) // default: no-op (NoCache, lazy impls)
    }
}

/// Metadata stored alongside each cached projection value.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CacheMeta {
    /// Global sequence watermark at the time the value was cached.
    pub watermark: u64,
    /// Wall-clock timestamp (microseconds since epoch) when the value was cached.
    pub cached_at_us: i64,
}

/// Byte layout: value bytes followed by 16 bytes of metadata (watermark u64 LE + cached_at_us i64 LE).
const CACHE_META_SIZE: usize = 16;

impl CacheMeta {
    /// Encode value + metadata into a single byte buffer for cache storage.
    pub(crate) fn encode_with_value(&self, value: &[u8]) -> Vec<u8> {
        let mut buf = Vec::with_capacity(value.len() + CACHE_META_SIZE);
        buf.extend_from_slice(value);
        buf.extend_from_slice(&self.watermark.to_le_bytes());
        buf.extend_from_slice(&self.cached_at_us.to_le_bytes());
        buf
    }

    /// Decode value + metadata from a cache-stored byte buffer.
    pub(crate) fn decode_from_bytes(bytes: &[u8]) -> Result<(Vec<u8>, Self), StoreError> {
        if bytes.len() < CACHE_META_SIZE {
            return Err(StoreError::cache_msg("corrupt cache metadata: too short"));
        }
        let (value, meta_bytes) = bytes.split_at(bytes.len() - CACHE_META_SIZE);
        let watermark = u64::from_le_bytes(
            meta_bytes[..8]
                .try_into()
                .map_err(|_| StoreError::cache_msg("corrupt cache metadata"))?,
        );
        let cached_at_us = i64::from_le_bytes(
            meta_bytes[8..16]
                .try_into()
                .map_err(|_| StoreError::cache_msg("corrupt cache metadata"))?,
        );
        Ok((
            value.to_vec(),
            Self {
                watermark,
                cached_at_us,
            },
        ))
    }
}

/// Controls how stale a cached projection may be when returned by `project()`.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum Freshness {
    /// Always replay from the current head; never return a stale cached value.
    Consistent,
    /// Return a cached value if it is no older than `max_stale_ms` milliseconds.
    BestEffort {
        /// Maximum age in milliseconds a cached value may have before forcing a replay.
        max_stale_ms: u64,
    },
}

/// No-op cache backend. Every `project()` call replays events from segments; nothing is stored.
pub struct NoCache;

impl ProjectionCache for NoCache {
    fn capabilities(&self) -> CacheCapabilities {
        CacheCapabilities::none()
    }

    fn get(&self, _key: &[u8]) -> Result<Option<(Vec<u8>, CacheMeta)>, StoreError> {
        Ok(None) // always miss — forces replay
    }

    fn put(&self, _key: &[u8], _value: &[u8], _meta: CacheMeta) -> Result<(), StoreError> {
        Ok(()) // no-op
    }

    fn delete_prefix(&self, _prefix: &[u8]) -> Result<u64, StoreError> {
        Ok(0) // nothing to delete
    }

    fn sync(&self) -> Result<(), StoreError> {
        Ok(()) // nothing to sync
    }
}

/// Projection cache backed by the embedded redb database. Requires the `redb` feature.
#[cfg(feature = "redb")]
pub struct RedbCache {
    db: redb::Database,
}

#[cfg(feature = "redb")]
const CACHE_TABLE: redb::TableDefinition<&[u8], &[u8]> =
    redb::TableDefinition::new("projection_cache");

#[cfg(feature = "redb")]
impl RedbCache {
    /// Open (or create) a redb cache database at the given path.
    ///
    /// # Errors
    /// Returns `StoreError::CacheFailed` if the redb database cannot be created or opened.
    pub fn open(path: impl AsRef<std::path::Path>) -> Result<Self, StoreError> {
        let db = redb::Database::create(path.as_ref())
            .map_err(|e| StoreError::CacheFailed(Box::new(e)))?;
        Ok(Self { db })
    }
}

#[cfg(feature = "redb")]
impl ProjectionCache for RedbCache {
    fn capabilities(&self) -> CacheCapabilities {
        CacheCapabilities::none()
    }

    fn get(&self, key: &[u8]) -> Result<Option<(Vec<u8>, CacheMeta)>, StoreError> {
        let txn = self
            .db
            .begin_read()
            .map_err(|e| StoreError::CacheFailed(Box::new(e)))?;
        let table = txn
            .open_table(CACHE_TABLE)
            .map_err(|e| StoreError::CacheFailed(Box::new(e)))?;
        match table.get(key) {
            Ok(Some(guard)) => {
                let bytes = guard.value().to_vec();
                let (value, meta) = CacheMeta::decode_from_bytes(&bytes)?;
                Ok(Some((value, meta)))
            }
            Ok(None) => Ok(None),
            Err(e) => Err(StoreError::CacheFailed(Box::new(e))),
        }
    }

    fn put(&self, key: &[u8], value: &[u8], meta: CacheMeta) -> Result<(), StoreError> {
        let buf = meta.encode_with_value(value);
        let txn = self
            .db
            .begin_write()
            .map_err(|e| StoreError::CacheFailed(Box::new(e)))?;
        {
            let mut table = txn
                .open_table(CACHE_TABLE)
                .map_err(|e| StoreError::CacheFailed(Box::new(e)))?;
            table
                .insert(key, buf.as_slice())
                .map_err(|e| StoreError::CacheFailed(Box::new(e)))?;
        }
        txn.commit()
            .map_err(|e| StoreError::CacheFailed(Box::new(e)))?;
        Ok(())
    }

    fn delete_prefix(&self, prefix: &[u8]) -> Result<u64, StoreError> {
        use redb::ReadableTable;
        // redb has no built-in delete_prefix. Iterate range + collect keys + delete.
        let txn = self
            .db
            .begin_write()
            .map_err(|e| StoreError::CacheFailed(Box::new(e)))?;
        let mut count = 0u64;
        {
            let mut table = txn
                .open_table(CACHE_TABLE)
                .map_err(|e| StoreError::CacheFailed(Box::new(e)))?;
            // Range: prefix..prefix_end. Compute the exclusive upper bound
            // by incrementing the last byte (with carry).
            // When no successor exists (all 0xFF or empty prefix), scan to end.
            let end = prefix_successor(prefix);
            let keys: Vec<Vec<u8>> = if let Some(end) = end {
                table
                    .range(prefix..end.as_slice())
                    .map_err(|e| StoreError::CacheFailed(Box::new(e)))?
                    .filter_map(|r| match r {
                        Ok(v) => Some(v),
                        Err(e) => {
                            tracing::warn!("cache iteration error (skipping row): {e}");
                            None
                        }
                    })
                    .map(|(k, _)| k.value().to_vec())
                    .collect()
            } else {
                // No upper bound — scan from prefix to end of table
                table
                    .range(prefix..)
                    .map_err(|e| StoreError::CacheFailed(Box::new(e)))?
                    .filter_map(|r| match r {
                        Ok(v) => Some(v),
                        Err(e) => {
                            tracing::warn!("cache iteration error (skipping row): {e}");
                            None
                        }
                    })
                    .map(|(k, _)| k.value().to_vec())
                    .collect()
            };
            for key in &keys {
                table
                    .remove(key.as_slice())
                    .map_err(|e| StoreError::CacheFailed(Box::new(e)))?;
                count += 1;
            }
        }
        txn.commit()
            .map_err(|e| StoreError::CacheFailed(Box::new(e)))?;
        Ok(count)
    }

    fn sync(&self) -> Result<(), StoreError> {
        Ok(()) // redb commits are durable by default
    }
}

/// Projection cache backed by LMDB via the heed crate. Requires the `lmdb` feature.
#[cfg(feature = "lmdb")]
pub struct LmdbCache {
    env: heed::Env,
    db: heed::Database<heed::types::Bytes, heed::types::Bytes>,
}

#[cfg(feature = "lmdb")]
impl LmdbCache {
    /// Open (or create) an LMDB cache environment at the given path with the specified map size in bytes.
    ///
    /// # Errors
    /// Returns `StoreError::Io` if the directory cannot be created.
    /// Returns `StoreError::CacheFailed` if the LMDB environment cannot be opened or initialized.
    pub fn open(path: impl AsRef<std::path::Path>, map_size: usize) -> Result<Self, StoreError> {
        std::fs::create_dir_all(path.as_ref()).map_err(StoreError::Io)?;
        let env = open_lmdb_env(path.as_ref(), map_size)?;
        let mut wtxn = env
            .write_txn()
            .map_err(|e| StoreError::CacheFailed(Box::new(e)))?;
        let db = env
            .create_database(&mut wtxn, Some("projection_cache"))
            .map_err(|e| StoreError::CacheFailed(Box::new(e)))?;
        wtxn.commit()
            .map_err(|e| StoreError::CacheFailed(Box::new(e)))?;
        Ok(Self { env, db })
    }
}

#[cfg(feature = "lmdb")]
impl ProjectionCache for LmdbCache {
    fn capabilities(&self) -> CacheCapabilities {
        CacheCapabilities::none()
    }

    fn get(&self, key: &[u8]) -> Result<Option<(Vec<u8>, CacheMeta)>, StoreError> {
        let txn = self
            .env
            .read_txn()
            .map_err(|e| StoreError::CacheFailed(Box::new(e)))?;
        match self
            .db
            .get(&txn, key)
            .map_err(|e| StoreError::CacheFailed(Box::new(e)))?
        {
            Some(bytes) if bytes.len() >= CACHE_META_SIZE => {
                let (value, meta) = CacheMeta::decode_from_bytes(bytes)?;
                Ok(Some((value, meta)))
            }
            _ => Ok(None),
        }
    }

    fn put(&self, key: &[u8], value: &[u8], meta: CacheMeta) -> Result<(), StoreError> {
        let buf = meta.encode_with_value(value);
        let mut txn = self
            .env
            .write_txn()
            .map_err(|e| StoreError::CacheFailed(Box::new(e)))?;
        self.db
            .put(&mut txn, key, &buf)
            .map_err(|e| StoreError::CacheFailed(Box::new(e)))?;
        txn.commit()
            .map_err(|e| StoreError::CacheFailed(Box::new(e)))?;
        Ok(())
    }

    fn delete_prefix(&self, prefix: &[u8]) -> Result<u64, StoreError> {
        // heed does NOT have delete_prefix. Use prefix_iter_mut + del_current.
        let mut txn = self
            .env
            .write_txn()
            .map_err(|e| StoreError::CacheFailed(Box::new(e)))?;
        let mut iter = self
            .db
            .prefix_iter_mut(&mut txn, prefix)
            .map_err(|e| StoreError::CacheFailed(Box::new(e)))?;
        let mut count = 0u64;
        while iter
            .next()
            .transpose()
            .map_err(|e| StoreError::CacheFailed(Box::new(e)))?
            .is_some()
        {
            // SAFETY: The iterator has been advanced and the current entry is not
            // retained outside this loop body before deletion.
            unsafe {
                iter.del_current()
                    .map_err(|e| StoreError::CacheFailed(Box::new(e)))?;
            }
            count += 1;
        }
        drop(iter);
        txn.commit()
            .map_err(|e| StoreError::CacheFailed(Box::new(e)))?;
        Ok(count)
    }

    fn sync(&self) -> Result<(), StoreError> {
        self.env
            .force_sync()
            .map_err(|e| StoreError::CacheFailed(Box::new(e)))
    }
}

#[cfg(feature = "redb")]
/// Compute the exclusive upper bound for a prefix range scan.
/// Increments the last byte, carrying if 0xFF. Returns None if no finite
/// upper bound exists (all bytes 0xFF, or empty prefix) — caller must
/// use an unbounded range scan.
fn prefix_successor(prefix: &[u8]) -> Option<Vec<u8>> {
    if prefix.is_empty() {
        return None; // empty prefix matches everything — no upper bound
    }
    let mut end = prefix.to_vec();
    // Walk backwards, incrementing the last non-0xFF byte
    for i in (0..end.len()).rev() {
        if end[i] < 0xFF {
            end[i] += 1;
            end.truncate(i + 1);
            return Some(end);
        }
    }
    // All bytes are 0xFF — no finite successor exists
    None
}

#[cfg(feature = "lmdb")]
/// Open an LMDB environment at `path` with the given `map_size`.
///
/// # Safety contract
///
/// LMDB requires that only one environment is open per directory within a process.
/// `LmdbCache` upholds this by owning the `heed::Env` and requiring callers to
/// provide a unique path per cache instance. Opening two `LmdbCache` instances
/// against the same directory is undefined behavior at the LMDB level.
fn open_lmdb_env(path: &std::path::Path, map_size: usize) -> Result<heed::Env, StoreError> {
    // SAFETY: LmdbCache owns the environment and opens exactly one LMDB environment
    // per on-disk cache path within the current process. Callers do not share the
    // same directory across multiple open environments.
    unsafe {
        heed::EnvOpenOptions::new()
            .map_size(map_size)
            .max_dbs(1)
            .open(path)
            .map_err(|e| StoreError::CacheFailed(Box::new(e)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_meta_encode_decode_roundtrip() {
        let meta = CacheMeta {
            watermark: 42,
            cached_at_us: 1_700_000_000_000,
        };
        let value = b"hello world";
        let encoded = meta.encode_with_value(value);
        let (decoded_value, decoded_meta) =
            CacheMeta::decode_from_bytes(&encoded).expect("decode should succeed");
        assert_eq!(decoded_value, value);
        assert_eq!(decoded_meta.watermark, 42);
        assert_eq!(decoded_meta.cached_at_us, 1_700_000_000_000);
    }

    #[test]
    fn cache_meta_decode_rejects_short_buffer() {
        let short = [0u8; 8];
        let result = CacheMeta::decode_from_bytes(&short);
        assert!(result.is_err());
    }

    #[test]
    fn cache_meta_roundtrip_empty_value() {
        let meta = CacheMeta {
            watermark: 0,
            cached_at_us: 0,
        };
        let encoded = meta.encode_with_value(b"");
        let (decoded_value, decoded_meta) =
            CacheMeta::decode_from_bytes(&encoded).expect("decode should succeed");
        assert!(decoded_value.is_empty());
        assert_eq!(decoded_meta.watermark, 0);
        assert_eq!(decoded_meta.cached_at_us, 0);
    }
}
