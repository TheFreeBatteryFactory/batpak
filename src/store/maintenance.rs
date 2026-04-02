use crate::coordinate::Coordinate;
use crate::event::{EventKind, StoredEvent};
use crate::store::index::{DiskPos, IndexEntry};
use crate::store::reader;
use crate::store::segment::{self, Active, FramePayload};
use crate::store::{
    CompactionConfig, CompactionStrategy, Store, StoreDiagnostics, StoreError, StoreStats,
};

pub(crate) fn sync(store: &Store) -> Result<(), StoreError> {
    tracing::debug!(target: "batpak::flow", flow = "sync");
    let (tx, rx) = flume::bounded(1);
    store
        .writer
        .tx
        .send(crate::store::writer::WriterCommand::Sync { respond: tx })
        .map_err(|_| StoreError::WriterCrashed)?;
    rx.recv().map_err(|_| StoreError::WriterCrashed)?
}

pub(crate) fn snapshot(store: &Store, dest: &std::path::Path) -> Result<(), StoreError> {
    tracing::debug!(
        target: "batpak::flow",
        flow = "snapshot",
        destination = %dest.display()
    );
    sync(store)?;
    std::fs::create_dir_all(dest).map_err(StoreError::Io)?;
    let entries = std::fs::read_dir(&store.config.data_dir).map_err(StoreError::Io)?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path
            .extension()
            .map(|ext| ext == segment::SEGMENT_EXTENSION)
            .unwrap_or(false)
        {
            let dest_path = dest.join(entry.file_name());
            std::fs::copy(&path, &dest_path).map_err(StoreError::Io)?;
        }
    }
    Ok(())
}

pub(crate) fn compact(
    store: &Store,
    config: &CompactionConfig,
) -> Result<segment::CompactionResult, StoreError> {
    tracing::debug!(target: "batpak::flow", flow = "compact");
    sync(store)?;

    let active_segment_id = std::fs::read_dir(&store.config.data_dir)
        .map_err(StoreError::Io)?
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let path = entry.path();
            if path
                .extension()
                .map(|ext| ext == segment::SEGMENT_EXTENSION)
                .unwrap_or(false)
            {
                path.file_stem()?.to_str()?.parse::<u64>().ok()
            } else {
                None
            }
        })
        .max()
        .unwrap_or(0);

    let mut sealed: Vec<(u64, std::path::PathBuf)> = std::fs::read_dir(&store.config.data_dir)
        .map_err(StoreError::Io)?
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let path = entry.path();
            let ext_ok = path
                .extension()
                .map(|ext| ext == segment::SEGMENT_EXTENSION)
                .unwrap_or(false);
            if !ext_ok {
                return None;
            }
            let seg_id = path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .and_then(|stem| stem.parse::<u64>().ok())?;
            if seg_id >= active_segment_id {
                return None;
            }
            Some((seg_id, path))
        })
        .collect();
    sealed.sort_by_key(|(id, _)| *id);

    if sealed.len() < config.min_segments {
        return Ok(segment::CompactionResult {
            segments_removed: 0,
            bytes_reclaimed: 0,
        });
    }

    let merged_id = sealed[0].0;
    let merged_path = store
        .config
        .data_dir
        .join(segment::segment_filename(merged_id));
    let mut compact_source_path = None;

    for (seg_id, _) in &sealed {
        store.reader.evict_segment(*seg_id);
    }

    if let Some((_, source_path)) = sealed.iter_mut().find(|(seg_id, _)| *seg_id == merged_id) {
        let temp_source_path = store.config.data_dir.join(format!(
            "{merged_id:06}.{}.compact-src",
            segment::SEGMENT_EXTENSION
        ));
        let _ = std::fs::remove_file(&temp_source_path);
        std::fs::rename(&*source_path, &temp_source_path).map_err(StoreError::Io)?;
        *source_path = temp_source_path.clone();
        compact_source_path = Some(temp_source_path);
    }

    let _ = std::fs::remove_file(&merged_path);
    let mut merged_segment = segment::Segment::<Active>::create(&store.config.data_dir, merged_id)?;
    match &config.strategy {
        CompactionStrategy::Merge => {
            for (_, path) in &sealed {
                merged_segment.append_frames_from_segment(path)?;
            }
        }
        CompactionStrategy::Retention(predicate) => {
            let mut all_events: Vec<reader::ScannedEntry> = Vec::new();
            for (_, path) in &sealed {
                all_events.extend(store.reader.scan_segment(path)?);
            }
            for entry in all_events {
                let coord = Coordinate::new(&entry.entity, &entry.scope)?;
                let stored = StoredEvent {
                    coordinate: coord,
                    event: entry.event.clone(),
                };
                if predicate(&stored) {
                    let frame_payload = FramePayload {
                        event: entry.event,
                        entity: entry.entity,
                        scope: entry.scope,
                    };
                    let frame = segment::frame_encode(&frame_payload)?;
                    merged_segment.write_frame(&frame)?;
                }
            }
        }
        CompactionStrategy::Tombstone(predicate) => {
            let mut all_events: Vec<reader::ScannedEntry> = Vec::new();
            for (_, path) in &sealed {
                all_events.extend(store.reader.scan_segment(path)?);
            }
            let tombstone_kind = EventKind::TOMBSTONE;
            for mut entry in all_events {
                let coord = Coordinate::new(&entry.entity, &entry.scope)?;
                let stored = StoredEvent {
                    coordinate: coord,
                    event: entry.event.clone(),
                };
                if !predicate(&stored) {
                    entry.event.header.event_kind = tombstone_kind;
                }
                let frame_payload = FramePayload {
                    event: entry.event,
                    entity: entry.entity,
                    scope: entry.scope,
                };
                let frame = segment::frame_encode(&frame_payload)?;
                merged_segment.write_frame(&frame)?;
            }
        }
    }

    merged_segment.sync_with_mode(&store.config.sync_mode)?;
    let _sealed_segment = merged_segment.seal();

    let mut bytes_reclaimed = 0_u64;
    let mut segments_removed = 0_usize;
    for (_, path) in &sealed {
        if let Ok(meta) = std::fs::metadata(path) {
            bytes_reclaimed += meta.len();
        }
        std::fs::remove_file(path).map_err(StoreError::Io)?;
        segments_removed += 1;
    }

    if let Some(temp_source_path) = compact_source_path {
        let _ = std::fs::remove_file(temp_source_path);
    }

    sync(store)?;
    store.index.clear();
    let mut remaining: Vec<std::fs::DirEntry> = std::fs::read_dir(&store.config.data_dir)
        .map_err(StoreError::Io)?
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .path()
                .extension()
                .map(|ext| ext == segment::SEGMENT_EXTENSION)
                .unwrap_or(false)
        })
        .collect();
    remaining.sort_by_key(|entry| entry.file_name());

    for dir_entry in &remaining {
        let scanned = store.reader.scan_segment_index(&dir_entry.path())?;
        for scanned_entry in scanned {
            let coord = Coordinate::new(&scanned_entry.entity, &scanned_entry.scope)?;
            let clock = scanned_entry.header.position.sequence;
            let entry = IndexEntry {
                event_id: scanned_entry.header.event_id,
                correlation_id: scanned_entry.header.correlation_id,
                causation_id: scanned_entry.header.causation_id,
                coord,
                kind: scanned_entry.header.event_kind,
                wall_ms: scanned_entry.header.position.wall_ms,
                clock,
                hash_chain: scanned_entry.hash_chain,
                disk_pos: DiskPos {
                    segment_id: scanned_entry.segment_id,
                    offset: scanned_entry.offset,
                    length: scanned_entry.length,
                },
                global_sequence: store.index.global_sequence(),
            };
            store.index.insert(entry);
        }
    }

    sync(store)?;

    Ok(segment::CompactionResult {
        segments_removed,
        bytes_reclaimed,
    })
}

pub(crate) fn close(store: Store) -> Result<(), StoreError> {
    tracing::debug!(target: "batpak::flow", flow = "close");
    let (tx, rx) = flume::bounded(1);
    store
        .writer
        .tx
        .send(crate::store::writer::WriterCommand::Shutdown { respond: tx })
        .map_err(|_| StoreError::WriterCrashed)?;
    let result = rx.recv().map_err(|_| StoreError::WriterCrashed)?;
    drop(store);
    result
}

pub(crate) fn stats(store: &Store) -> StoreStats {
    StoreStats {
        event_count: store.index.len(),
        global_sequence: store.index.global_sequence(),
    }
}

pub(crate) fn diagnostics(store: &Store) -> StoreDiagnostics {
    StoreDiagnostics {
        event_count: store.index.len(),
        global_sequence: store.index.global_sequence(),
        data_dir: store.config.data_dir.clone(),
        segment_max_bytes: store.config.segment_max_bytes,
        fd_budget: store.config.fd_budget,
        restart_policy: store.config.restart_policy.clone(),
    }
}
