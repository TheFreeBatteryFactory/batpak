use crate::coordinate::Coordinate;
use crate::store::index::{DiskPos, IndexEntry, StoreIndex};
use crate::store::reader::Reader;
use crate::store::segment;
use crate::store::StoreError;
use rayon::prelude::*;
use std::path::Path;

/// Open the index using the fastest available path:
/// 1. Try loading a checkpoint file → if valid, restore from it + replay tail segments.
/// 2. Fall back to full segment scan if checkpoint is missing, corrupt, or stale.
pub(crate) fn open_index(
    index: &StoreIndex,
    reader: &Reader,
    data_dir: &Path,
    enable_checkpoint: bool,
    enable_mmap_index: bool,
) -> Result<(), StoreError> {
    if enable_mmap_index {
        if let Some((watermark, stored_allocator)) =
            crate::store::mmap_index::try_restore_mmap_index(index, data_dir)
        {
            tracing::info!(
                "mmap index loaded: watermark segment {} offset {}, allocator {}",
                watermark.watermark_segment_id,
                watermark.watermark_offset,
                stored_allocator,
            );
            replay_tail_segments(index, reader, data_dir, &watermark)?;
            restore_cancelled_visibility_ranges(index, data_dir);
            return Ok(());
        }
        tracing::debug!("no valid mmap index, trying checkpoint path");
    }
    if enable_checkpoint {
        if let Some((entries, interner_strings, watermark, stored_allocator)) =
            crate::store::checkpoint::try_load_checkpoint(data_dir)
        {
            tracing::info!(
                "checkpoint v2 loaded: {} entries, {} interner strings, watermark segment {} offset {}, allocator {}",
                entries.len(),
                interner_strings.len(),
                watermark.watermark_segment_id,
                watermark.watermark_offset,
                stored_allocator,
            );
            crate::store::checkpoint::restore_from_checkpoint(
                index,
                entries,
                &interner_strings,
                stored_allocator,
            )?;
            // Replay segments newer than the watermark.
            replay_tail_segments(index, reader, data_dir, &watermark)?;
            restore_cancelled_visibility_ranges(index, data_dir);
            return Ok(());
        }
        tracing::debug!("no valid checkpoint, performing full index rebuild");
    }
    rebuild_from_segments(index, reader, data_dir)?;
    restore_cancelled_visibility_ranges(index, data_dir);
    Ok(())
}

pub(crate) fn restore_cancelled_visibility_ranges(index: &StoreIndex, data_dir: &Path) {
    if let Some(ranges) = crate::store::visibility_ranges::try_load_cancelled_ranges(data_dir) {
        index.restore_cancelled_visibility_ranges(ranges);
    }
}

fn segment_paths(data_dir: &Path) -> Result<Vec<(u64, std::path::PathBuf)>, StoreError> {
    let mut entries: Vec<(u64, std::path::PathBuf)> = std::fs::read_dir(data_dir)?
        .filter_map(|e| e.ok())
        .filter_map(|entry| {
            let path = entry.path();
            let is_segment = path
                .extension()
                .map(|ext| ext == segment::SEGMENT_EXTENSION)
                .unwrap_or(false);
            if !is_segment {
                return None;
            }
            let segment_id = path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .and_then(|stem| stem.parse::<u64>().ok())?;
            Some((segment_id, path))
        })
        .collect();
    entries.sort_by_key(|(segment_id, _)| *segment_id);
    Ok(entries)
}

fn read_sealed_sidx_entries_parallel(
    sealed_segments: &[(u64, std::path::PathBuf)],
) -> Option<Vec<crate::store::reader::ScannedIndexEntry>> {
    let per_segment: Result<Vec<_>, StoreError> = sealed_segments
        .par_iter()
        .map(|(segment_id, path)| scanned_entries_from_sidx_footer(*segment_id, path))
        .collect();

    match per_segment {
        Ok(mut batches) => {
            let mut flat = Vec::new();
            for batch in batches.drain(..) {
                flat.extend(batch);
            }
            flat.sort_by_key(|entry| entry.global_sequence.unwrap_or(0));
            Some(flat)
        }
        Err(error) => {
            tracing::warn!(
                target: "batpak::rebuild",
                error = %error,
                "parallel SIDX rebuild unavailable; falling back to sequential scan"
            );
            None
        }
    }
}

fn scanned_entries_from_sidx_footer(
    segment_id: u64,
    path: &Path,
) -> Result<Vec<crate::store::reader::ScannedIndexEntry>, StoreError> {
    match crate::store::sidx::read_footer(path) {
        Ok(Some((entries, strings))) => {
            let mut scanned = Vec::with_capacity(entries.len());
            for entry in entries {
                let kind = crate::store::sidx::raw_to_kind(entry.kind);
                if kind == crate::event::EventKind::SYSTEM_BATCH_BEGIN
                    || kind == crate::event::EventKind::SYSTEM_BATCH_COMMIT
                {
                    continue;
                }
                let entity = strings
                    .get(entry.entity_idx as usize)
                    .cloned()
                    .ok_or_else(|| StoreError::ser_msg("SIDX entity_idx out of range"))?;
                let scope = strings
                    .get(entry.scope_idx as usize)
                    .cloned()
                    .ok_or_else(|| StoreError::ser_msg("SIDX scope_idx out of range"))?;
                scanned.push(crate::store::reader::ScannedIndexEntry {
                    header: crate::event::EventHeader::from_sidx(
                        entry.event_id,
                        entry.correlation_id,
                        (entry.causation_id != 0).then_some(entry.causation_id),
                        entry.wall_ms,
                        entry.clock,
                        kind,
                    ),
                    entity,
                    scope,
                    hash_chain: crate::event::HashChain {
                        prev_hash: entry.prev_hash,
                        event_hash: entry.event_hash,
                    },
                    segment_id,
                    offset: entry.frame_offset,
                    length: entry.frame_length,
                    global_sequence: Some(entry.global_sequence),
                });
            }
            Ok(scanned)
        }
        Ok(None) => Err(StoreError::ser_msg(
            "sealed segment missing SIDX footer during parallel rebuild",
        )),
        Err(error) => Err(error),
    }
}

#[cfg(test)]
fn read_sealed_sidx_entries_sequential(
    sealed_segments: &[(u64, std::path::PathBuf)],
) -> Result<Vec<crate::store::reader::ScannedIndexEntry>, StoreError> {
    let mut flat = Vec::new();
    for (segment_id, path) in sealed_segments {
        flat.extend(scanned_entries_from_sidx_footer(*segment_id, path)?);
    }
    flat.sort_by_key(|entry| entry.global_sequence.unwrap_or(0));
    Ok(flat)
}

/// Build an `IndexEntry` from a `ScannedIndexEntry`, sourcing `global_sequence`
/// from the SIDX footer if available, otherwise asking the cursor to synthesize
/// the next free slot. This keeps sparse `global_sequence` values from disk
/// preserved verbatim across cold-start rebuilds.
fn entry_from_scan(
    index: &StoreIndex,
    cursor: &mut crate::store::index::ReplayCursor<'_>,
    se: crate::store::reader::ScannedIndexEntry,
) -> Result<IndexEntry, StoreError> {
    let coord = Coordinate::new(&se.entity, &se.scope)?;
    let entity_id = index.interner.intern(&se.entity);
    let scope_id = index.interner.intern(&se.scope);
    let clock = se.header.position.sequence;
    // SIDX-stored sequence wins; otherwise synthesize from the cursor's
    // running maximum (active segment / footerless slow path).
    let global_sequence = se
        .global_sequence
        .unwrap_or_else(|| cursor.synthesize_next());
    Ok(IndexEntry {
        event_id: se.header.event_id,
        correlation_id: se.header.correlation_id,
        causation_id: se.header.causation_id,
        coord,
        entity_id,
        scope_id,
        kind: se.header.event_kind,
        wall_ms: se.header.position.wall_ms,
        clock,
        hash_chain: se.hash_chain,
        disk_pos: DiskPos {
            segment_id: se.segment_id,
            offset: se.offset,
            length: se.length,
        },
        global_sequence,
    })
}

/// Replay only segments with ID > watermark, or frames at offset >= watermark_offset
/// within the watermark segment itself.
fn replay_tail_segments(
    index: &StoreIndex,
    reader: &Reader,
    data_dir: &Path,
    watermark: &crate::store::checkpoint::WatermarkInfo,
) -> Result<(), StoreError> {
    let entries = segment_paths(data_dir)?;

    // Cross-segment batch recovery state persists across segment scans.
    let mut batch_state = crate::store::reader::BatchRecoveryState::default();

    let mut cursor = index.begin_replay();
    // Tail replay continues from wherever the checkpoint restore left the
    // allocator — pass the current value as the synthesis floor so any
    // synthesized sequences advance from there.
    let allocator_floor = index.global_sequence();

    let scan_result = (|| -> Result<(), StoreError> {
        for (seg_id, path) in &entries {
            if *seg_id < watermark.watermark_segment_id {
                continue; // Already in checkpoint
            }

            reader.scan_segment_index_into(path, Some(&mut batch_state), |se| {
                // Skip frames already in the checkpoint
                if *seg_id == watermark.watermark_segment_id
                    && se.offset < watermark.watermark_offset
                {
                    return Ok(());
                }
                let entry = entry_from_scan(index, &mut cursor, se)?;
                cursor.insert(entry);
                Ok(())
            })?;
        }
        Ok(())
    })();

    match scan_result {
        Ok(()) => {
            // All tail entries are now in the index. Restore allocator (preserving
            // both the checkpoint allocator floor and any sparse SIDX-preserved
            // sequences) and publish atomically.
            cursor.commit(allocator_floor);
            Ok(())
        }
        Err(e) => {
            cursor.abort();
            Err(e)
        }
    }
}

/// Scan all segment files in `data_dir`, rebuild the in-memory index from their contents.
/// Used by both cold-start (`Store::open_with_cache`) and post-compaction index rebuild.
/// Handles cross-segment batch recovery using BatchRecoveryState.
pub(crate) fn rebuild_from_segments(
    index: &StoreIndex,
    reader: &Reader,
    data_dir: &Path,
) -> Result<(), StoreError> {
    let entries = segment_paths(data_dir)?;
    let configured_active_segment = reader.active_segment_id();
    let active_segment_id = (configured_active_segment != 0).then_some(configured_active_segment);

    let mut cursor = index.begin_replay();

    let scan_result = (|| -> Result<(), StoreError> {
        let sealed_segments: Vec<_> = entries
            .iter()
            .filter(|(segment_id, _)| active_segment_id.is_none_or(|active| *segment_id < active))
            .cloned()
            .collect();
        if !sealed_segments.is_empty() {
            if let Some(scanned) = read_sealed_sidx_entries_parallel(&sealed_segments) {
                for se in scanned {
                    let entry = entry_from_scan(index, &mut cursor, se)?;
                    cursor.insert(entry);
                }
            } else {
                let mut batch_state = crate::store::reader::BatchRecoveryState::default();
                for (_, path) in &sealed_segments {
                    reader.scan_segment_index_into(path, Some(&mut batch_state), |se| {
                        let entry = entry_from_scan(index, &mut cursor, se)?;
                        cursor.insert(entry);
                        Ok(())
                    })?;
                }
            }
        }

        let mut batch_state = crate::store::reader::BatchRecoveryState::default();
        for (segment_id, path) in &entries {
            if Some(*segment_id) != active_segment_id {
                continue;
            }
            reader.scan_segment_index_into(path, Some(&mut batch_state), |se| {
                let entry = entry_from_scan(index, &mut cursor, se)?;
                cursor.insert(entry);
                Ok(())
            })?;
        }
        Ok(())
    })();

    match scan_result {
        Ok(()) => {
            // Full rebuild complete. No allocator hint — preserve SIDX
            // sequences as-is and advance the allocator past the maximum seen.
            cursor.commit(0);
            Ok(())
        }
        Err(e) => {
            cursor.abort();
            Err(e)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prelude::*;
    use tempfile::TempDir;

    #[derive(Clone, Debug, Eq, PartialEq)]
    struct ScanSummaryRow {
        event_id: u128,
        entity: String,
        scope: String,
        category: u8,
        type_id: u16,
        global_sequence: u64,
        offset: u64,
        length: u32,
    }

    fn rotating_store_config(dir: &TempDir) -> StoreConfig {
        StoreConfig::new(dir.path())
            .with_segment_max_bytes(512)
            .with_sync_every_n_events(1)
            .with_enable_checkpoint(false)
            .with_enable_mmap_index(false)
    }

    fn scanned_summary(entries: &[crate::store::reader::ScannedIndexEntry]) -> Vec<ScanSummaryRow> {
        entries
            .iter()
            .map(|entry| ScanSummaryRow {
                event_id: entry.header.event_id,
                entity: entry.entity.clone(),
                scope: entry.scope.clone(),
                category: entry.header.event_kind.category(),
                type_id: entry.header.event_kind.type_id(),
                global_sequence: entry.global_sequence.unwrap_or(0),
                offset: entry.offset,
                length: entry.length,
            })
            .collect()
    }

    #[test]
    fn parallel_sidx_footer_read_matches_sequential_footer_read() {
        let dir = TempDir::new().expect("temp dir");
        let store = Store::open(rotating_store_config(&dir)).expect("open store");
        let coord = Coordinate::new("entity:sidx", "scope:rebuild").expect("coord");
        let kind = EventKind::custom(0xF, 9);
        let payload = serde_json::json!({
            "blob": "payload that forces rapid segment rotation and sealed footer generation"
        });

        for n in 0..64u32 {
            store
                .append(
                    &coord,
                    kind,
                    &serde_json::json!({"n": n, "payload": payload}),
                )
                .expect("append");
        }
        store.close().expect("close store");

        let entries = segment_paths(dir.path()).expect("segment paths");
        let active_segment = entries
            .iter()
            .map(|(segment_id, _)| *segment_id)
            .max()
            .expect("at least one segment");
        let sealed_segments: Vec<_> = entries
            .into_iter()
            .filter(|(segment_id, _)| *segment_id < active_segment)
            .collect();

        assert!(
            !sealed_segments.is_empty(),
            "PROPERTY: tiny segments should produce at least one sealed segment with an SIDX footer."
        );

        let parallel = read_sealed_sidx_entries_parallel(&sealed_segments)
            .expect("parallel SIDX footer read should succeed");
        let sequential = read_sealed_sidx_entries_sequential(&sealed_segments)
            .expect("sequential SIDX footer read should succeed");

        assert_eq!(
            scanned_summary(&parallel),
            scanned_summary(&sequential),
            "PROPERTY: parallel SIDX footer rebuild must match sequential footer semantics exactly."
        );
    }
}
