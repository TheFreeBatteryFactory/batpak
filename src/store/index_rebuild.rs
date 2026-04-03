use crate::coordinate::Coordinate;
use crate::store::index::{DiskPos, IndexEntry, StoreIndex};
use crate::store::reader::Reader;
use crate::store::segment;
use crate::store::StoreError;
use std::path::Path;

/// Scan all segment files in `data_dir`, rebuild the in-memory index from their contents.
/// Used by both cold-start (`Store::open_with_cache`) and post-compaction index rebuild.
pub(crate) fn rebuild_from_segments(
    index: &StoreIndex,
    reader: &Reader,
    data_dir: &Path,
) -> Result<(), StoreError> {
    let mut entries: Vec<std::fs::DirEntry> = std::fs::read_dir(data_dir)?
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.path()
                .extension()
                .map(|ext| ext == segment::SEGMENT_EXTENSION)
                .unwrap_or(false)
        })
        .collect();
    entries.sort_by_key(|e| e.file_name());

    for dir_entry in &entries {
        let scanned = reader.scan_segment_index(&dir_entry.path())?;
        for se in scanned {
            let coord = Coordinate::new(&se.entity, &se.scope)?;
            let clock = se.header.position.sequence;
            let entry = IndexEntry {
                event_id: se.header.event_id,
                correlation_id: se.header.correlation_id,
                causation_id: se.header.causation_id,
                coord,
                kind: se.header.event_kind,
                wall_ms: se.header.position.wall_ms,
                clock,
                hash_chain: se.hash_chain,
                disk_pos: DiskPos {
                    segment_id: se.segment_id,
                    offset: se.offset,
                    length: se.length,
                },
                global_sequence: index.global_sequence(),
            };
            index.insert(entry);
        }
    }

    Ok(())
}
