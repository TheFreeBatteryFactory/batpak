use crate::store::StoreError;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct CacheCapabilities {
    pub supports_prefetch: bool,
}

impl CacheCapabilities {
    pub const fn none() -> Self {
        Self {
            supports_prefetch: false,
        }
    }

    pub const fn prefetch_hints() -> Self {
        Self {
            supports_prefetch: true,
        }
    }
}

// ProjectionCache: trait for caching projected state.
// Three impls: NoCache (default), RedbCache (optional), LmdbCache (optional).
pub trait ProjectionCache: Send + Sync + 'static {
    fn capabilities(&self) -> CacheCapabilities;
    fn get(&self, key: &[u8]) -> Result<Option<(Vec<u8>, CacheMeta)>, StoreError>;
    fn put(&self, key: &[u8], value: &[u8], meta: CacheMeta) -> Result<(), StoreError>;
    fn delete_prefix(&self, prefix: &[u8]) -> Result<u64, StoreError>;
    fn sync(&self) -> Result<(), StoreError>;

    /// Hint that this key is likely to be requested soon. Implementations may
    /// pre-warm internal caches or pre-compute values. Default: no-op.
    fn prefetch(&self, _key: &[u8], _predicted_meta: CacheMeta) -> Result<(), StoreError> {
        Ok(()) // default: no-op (NoCache, lazy impls)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CacheMeta {
    pub watermark: u64,
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
            return Err(StoreError::CacheFailed(
                "corrupt cache metadata: too short".into(),
            ));
        }
        let (value, meta_bytes) = bytes.split_at(bytes.len() - CACHE_META_SIZE);
        let watermark = u64::from_le_bytes(
            meta_bytes[..8]
                .try_into()
                .map_err(|_| StoreError::CacheFailed("corrupt cache metadata".into()))?,
        );
        let cached_at_us = i64::from_le_bytes(
            meta_bytes[8..16]
                .try_into()
                .map_err(|_| StoreError::CacheFailed("corrupt cache metadata".into()))?,
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

#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum Freshness {
    Consistent,
    BestEffort { max_stale_ms: u64 },
}

// NoCache: default. Every read replays from segments. No state.
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

// RedbCache: backed by redb embedded database.
#[cfg(feature = "redb")]
pub struct RedbCache {
    db: redb::Database,
}

#[cfg(feature = "redb")]
const CACHE_TABLE: redb::TableDefinition<&[u8], &[u8]> =
    redb::TableDefinition::new("projection_cache");

#[cfg(feature = "redb")]
impl RedbCache {
    pub fn open(path: impl AsRef<std::path::Path>) -> Result<Self, StoreError> {
        let db = redb::Database::create(path.as_ref())
            .map_err(|e| StoreError::CacheFailed(e.to_string()))?;
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
            .map_err(|e| StoreError::CacheFailed(e.to_string()))?;
        let table = txn
            .open_table(CACHE_TABLE)
            .map_err(|e| StoreError::CacheFailed(e.to_string()))?;
        match table.get(key) {
            Ok(Some(guard)) => {
                let bytes = guard.value().to_vec();
                let (value, meta) = CacheMeta::decode_from_bytes(&bytes)?;
                Ok(Some((value, meta)))
            }
            Ok(None) => Ok(None),
            Err(e) => Err(StoreError::CacheFailed(e.to_string())),
        }
    }

    fn put(&self, key: &[u8], value: &[u8], meta: CacheMeta) -> Result<(), StoreError> {
        let buf = meta.encode_with_value(value);
        let txn = self
            .db
            .begin_write()
            .map_err(|e| StoreError::CacheFailed(e.to_string()))?;
        {
            let mut table = txn
                .open_table(CACHE_TABLE)
                .map_err(|e| StoreError::CacheFailed(e.to_string()))?;
            table
                .insert(key, buf.as_slice())
                .map_err(|e| StoreError::CacheFailed(e.to_string()))?;
        }
        txn.commit()
            .map_err(|e| StoreError::CacheFailed(e.to_string()))?;
        Ok(())
    }

    fn delete_prefix(&self, prefix: &[u8]) -> Result<u64, StoreError> {
        use redb::ReadableTable;
        // redb has no built-in delete_prefix. Iterate range + collect keys + delete.
        let txn = self
            .db
            .begin_write()
            .map_err(|e| StoreError::CacheFailed(e.to_string()))?;
        let mut count = 0u64;
        {
            let mut table = txn
                .open_table(CACHE_TABLE)
                .map_err(|e| StoreError::CacheFailed(e.to_string()))?;
            // Range: prefix..prefix_end. Compute the exclusive upper bound
            // by incrementing the last byte (with carry).
            // When no successor exists (all 0xFF or empty prefix), scan to end.
            let end = prefix_successor(prefix);
            let keys: Vec<Vec<u8>> = if let Some(end) = end {
                table
                    .range(prefix..end.as_slice())
                    .map_err(|e| StoreError::CacheFailed(e.to_string()))?
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
                    .map_err(|e| StoreError::CacheFailed(e.to_string()))?
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
                    .map_err(|e| StoreError::CacheFailed(e.to_string()))?;
                count += 1;
            }
        }
        txn.commit()
            .map_err(|e| StoreError::CacheFailed(e.to_string()))?;
        Ok(count)
    }

    fn sync(&self) -> Result<(), StoreError> {
        Ok(()) // redb commits are durable by default
    }
}

// LmdbCache: backed by LMDB via heed.
#[cfg(feature = "lmdb")]
pub struct LmdbCache {
    env: heed::Env,
    db: heed::Database<heed::types::Bytes, heed::types::Bytes>,
}

#[cfg(feature = "lmdb")]
impl LmdbCache {
    pub fn open(path: impl AsRef<std::path::Path>, map_size: usize) -> Result<Self, StoreError> {
        std::fs::create_dir_all(path.as_ref()).map_err(StoreError::Io)?;
        let env = open_lmdb_env(path.as_ref(), map_size)?;
        let mut wtxn = env
            .write_txn()
            .map_err(|e| StoreError::CacheFailed(e.to_string()))?;
        let db = env
            .create_database(&mut wtxn, Some("projection_cache"))
            .map_err(|e| StoreError::CacheFailed(e.to_string()))?;
        wtxn.commit()
            .map_err(|e| StoreError::CacheFailed(e.to_string()))?;
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
            .map_err(|e| StoreError::CacheFailed(e.to_string()))?;
        match self
            .db
            .get(&txn, key)
            .map_err(|e| StoreError::CacheFailed(e.to_string()))?
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
            .map_err(|e| StoreError::CacheFailed(e.to_string()))?;
        self.db
            .put(&mut txn, key, &buf)
            .map_err(|e| StoreError::CacheFailed(e.to_string()))?;
        txn.commit()
            .map_err(|e| StoreError::CacheFailed(e.to_string()))?;
        Ok(())
    }

    fn delete_prefix(&self, prefix: &[u8]) -> Result<u64, StoreError> {
        // heed does NOT have delete_prefix. Use prefix_iter_mut + del_current.
        let mut txn = self
            .env
            .write_txn()
            .map_err(|e| StoreError::CacheFailed(e.to_string()))?;
        let mut iter = self
            .db
            .prefix_iter_mut(&mut txn, prefix)
            .map_err(|e| StoreError::CacheFailed(e.to_string()))?;
        let mut count = 0u64;
        while iter
            .next()
            .transpose()
            .map_err(|e| StoreError::CacheFailed(e.to_string()))?
            .is_some()
        {
            // SAFETY: The iterator has been advanced and the current entry is not
            // retained outside this loop body before deletion.
            unsafe {
                iter.del_current()
                    .map_err(|e| StoreError::CacheFailed(e.to_string()))?;
            }
            count += 1;
        }
        drop(iter);
        txn.commit()
            .map_err(|e| StoreError::CacheFailed(e.to_string()))?;
        Ok(count)
    }

    fn sync(&self) -> Result<(), StoreError> {
        self.env
            .force_sync()
            .map_err(|e| StoreError::CacheFailed(e.to_string()))
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
            .map_err(|e| StoreError::CacheFailed(e.to_string()))
    }
}
