use crate::coordinate::CoordinateError;

/// StoreError: every error the store can produce.
/// [SPEC:src/store/mod.rs — StoreError variants]
#[derive(Debug)]
#[non_exhaustive]
pub enum StoreError {
    Io(std::io::Error),
    Coordinate(CoordinateError),
    Serialization(String),
    CrcMismatch {
        segment_id: u64,
        offset: u64,
    },
    CorruptSegment {
        segment_id: u64,
        detail: String,
    },
    NotFound(u128),
    SequenceMismatch {
        entity: String,
        expected: u32,
        actual: u32,
    },
    DuplicateEvent(u128),
    WriterCrashed,
    ShuttingDown,
    CacheFailed(String),
}

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "IO error: {e}"),
            Self::Coordinate(e) => write!(f, "coordinate error: {e}"),
            Self::Serialization(s) => write!(f, "serialization error: {s}"),
            Self::CrcMismatch { segment_id, offset } => {
                write!(f, "CRC mismatch in segment {segment_id} at offset {offset}")
            }
            Self::CorruptSegment { segment_id, detail } => {
                write!(f, "corrupt segment {segment_id}: {detail}")
            }
            Self::NotFound(id) => write!(f, "event {id:032x} not found"),
            Self::SequenceMismatch {
                entity,
                expected,
                actual,
            } => write!(
                f,
                "CAS failed for {entity}: expected seq {expected}, got {actual}"
            ),
            Self::DuplicateEvent(key) => write!(f, "duplicate idempotency key {key:032x}"),
            Self::WriterCrashed => write!(f, "writer thread crashed"),
            Self::ShuttingDown => write!(f, "store is shutting down"),
            Self::CacheFailed(s) => write!(f, "cache error: {s}"),
        }
    }
}

impl std::error::Error for StoreError {}

impl From<CoordinateError> for StoreError {
    fn from(e: CoordinateError) -> Self {
        Self::Coordinate(e)
    }
}

impl From<std::io::Error> for StoreError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}
