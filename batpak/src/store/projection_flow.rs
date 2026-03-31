use crate::event::EventSourced;
use crate::store::{projection, Freshness, Store, StoreError};

pub(crate) fn project<T>(
    store: &Store,
    entity: &str,
    freshness: &Freshness,
) -> Result<Option<T>, StoreError>
where
    T: EventSourced<serde_json::Value> + serde::Serialize + serde::de::DeserializeOwned,
{
    tracing::debug!(
        target: "batpak::flow",
        flow = "project",
        entity,
        freshness = match freshness {
            Freshness::Consistent => "consistent",
            Freshness::BestEffort { .. } => "best_effort",
        }
    );
    let entries = store.index.stream(entity);
    if entries.is_empty() {
        return Ok(None);
    }

    let watermark = entries.last().map(|e| e.global_sequence).unwrap_or(0);
    let cache_key = entity.as_bytes();
    let predicted_meta = projection::CacheMeta {
        watermark,
        cached_at_us: store.config.now_us(),
    };
    if store.cache.capabilities().supports_prefetch {
        if let Err(error) = store.cache.prefetch(cache_key, predicted_meta) {
            tracing::warn!("cache prefetch failed (non-fatal): {error}");
        }
    }

    if let Ok(Some((bytes, meta))) = store.cache.get(cache_key) {
        let is_fresh = match freshness {
            Freshness::Consistent => meta.watermark == watermark,
            Freshness::BestEffort { max_stale_ms } => {
                let age_us = store
                    .config
                    .now_us()
                    .saturating_sub(meta.cached_at_us)
                    .max(0);
                age_us < (*max_stale_ms as i64) * 1000
            }
        };
        if is_fresh {
            if let Ok(value) = serde_json::from_slice::<T>(&bytes) {
                return Ok(Some(value));
            }
        }
    }

    let mut events = Vec::with_capacity(entries.len());
    for entry in &entries {
        let stored = store.reader.read_entry(&entry.disk_pos)?;
        events.push(stored.event);
    }
    let result = T::from_events(&events);

    if let Some(ref value) = result {
        if let Ok(bytes) = serde_json::to_vec(value) {
            let meta = projection::CacheMeta {
                watermark,
                cached_at_us: store.config.now_us(),
            };
            if let Err(error) = store.cache.put(cache_key, &bytes, meta) {
                tracing::warn!("cache put failed (non-fatal): {error}");
            }
        }
    }

    Ok(result)
}
