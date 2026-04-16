use crate::coordinate::DagPosition;
use crate::event::EventKind;
use serde::{Deserialize, Serialize};

/// EventHeader: metadata for every event. Store generates this — users don't call new directly.
/// repr(C) for deterministic field ordering (NOT a wire format — msgpack handles serialization).
#[repr(C)]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventHeader {
    /// Globally unique identifier for this event, assigned by the store.
    #[serde(with = "crate::wire::u128_bytes")]
    pub event_id: u128,
    /// Groups related events that share a single originating request or saga.
    #[serde(with = "crate::wire::u128_bytes")]
    pub correlation_id: u128,
    /// Identifies the direct predecessor event that caused this one, if any.
    #[serde(with = "crate::wire::option_u128_bytes")]
    pub causation_id: Option<u128>,
    /// Wall-clock timestamp in microseconds when the event was appended.
    pub timestamp_us: i64,
    /// Logical position of this event within its stream DAG.
    pub position: DagPosition,
    /// Byte length of the serialized payload.
    pub payload_size: u32,
    /// Category discriminant describing what kind of domain event this is.
    pub event_kind: EventKind,
    /// Bit flags encoding delivery and transaction semantics.
    pub flags: u8,
    /// Content hash of the serialized payload. Enables automatic projection cache
    /// invalidation when event schemas evolve. Computed from payload bytes during
    /// writer step 5 (reuses the blake3 computation). [0u8; 32] when blake3 is off.
    #[serde(default)]
    pub content_hash: [u8; 32],
}

/// Flag bit constants for EventHeader.flags
/// Signals that the consumer must explicitly acknowledge this event before the next is delivered.
pub const FLAG_REQUIRES_ACK: u8 = 0x01;
/// Marks this event as part of an atomic transaction group.
pub const FLAG_TRANSACTIONAL: u8 = 0x02;
/// Marks this event as a replay of a previously persisted event rather than a live emission.
pub const FLAG_REPLAY: u8 = 0x08;

/// All fields extracted from an SIDX footer entry, bundled to avoid a long
/// positional argument list in [`EventHeader::from_sidx`].
pub(crate) struct SidxReconstructionFields {
    pub event_id: u128,
    pub correlation_id: u128,
    pub causation_id: Option<u128>,
    pub wall_ms: u64,
    pub clock: u32,
    pub lane: u32,
    pub depth: u32,
    pub event_kind: EventKind,
}

impl EventHeader {
    /// Constructs an `EventHeader` with all fields; flags and content hash default to zero.
    pub fn new(
        event_id: u128,
        correlation_id: u128,
        causation_id: Option<u128>,
        timestamp_us: i64,
        position: DagPosition,
        payload_size: u32,
        event_kind: EventKind,
    ) -> Self {
        Self {
            event_id,
            correlation_id,
            causation_id,
            timestamp_us,
            position,
            payload_size,
            event_kind,
            flags: 0,
            content_hash: [0u8; 32],
        }
    }

    /// Reconstruct an `EventHeader` from SIDX footer fields.
    /// Used during SIDX-accelerated cold start — only the fields stored in
    /// the SIDX entry are available; `timestamp_us`, `payload_size`, `flags`,
    /// and `content_hash` are set to defaults (0/empty).
    ///
    /// Note: `timestamp_us` is approximated as `wall_ms * 1000` (±999 µs).
    /// This approximation is intentional — `timestamp_us` is used only by the
    /// public `age_us()` display API, not for ordering. Internal ordering uses `wall_ms`.
    pub(crate) fn from_sidx(fields: SidxReconstructionFields) -> Self {
        Self {
            event_id: fields.event_id,
            correlation_id: fields.correlation_id,
            causation_id: fields.causation_id,
            timestamp_us: (fields.wall_ms * 1000) as i64, // approximate: ms → µs (±999 µs)
            position: DagPosition::with_hlc(fields.wall_ms, 0, fields.depth, fields.lane, fields.clock),
            payload_size: 0,
            event_kind: fields.event_kind,
            flags: 0,
            content_hash: [0u8; 32],
        }
    }

    /// Sets the flags byte on this header.
    pub fn with_flags(mut self, flags: u8) -> Self {
        self.flags = flags;
        self
    }

    /// Returns `true` if the consumer must acknowledge this event.
    pub fn requires_ack(&self) -> bool {
        self.flags & FLAG_REQUIRES_ACK != 0
    }

    /// Returns `true` if this event is part of a transaction.
    pub fn is_transactional(&self) -> bool {
        self.flags & FLAG_TRANSACTIONAL != 0
    }

    /// Returns `true` if this event is being replayed rather than emitted live.
    pub fn is_replay(&self) -> bool {
        self.flags & FLAG_REPLAY != 0
    }

    /// Returns the age of this event in microseconds relative to `now_us`.
    pub fn age_us(&self, now_us: i64) -> u64 {
        // saturating_sub + max(0) guarantees a non-negative i64; try_from is always Ok here.
        u64::try_from(now_us.saturating_sub(self.timestamp_us).max(0))
            .expect("invariant: .max(0) ensures value is non-negative")
    }
}

#[cfg(test)]
mod tests {
    use super::{EventHeader, SidxReconstructionFields};
    use crate::event::EventKind;

    #[test]
    fn from_sidx_preserves_non_root_lane_depth() {
        let header = EventHeader::from_sidx(SidxReconstructionFields {
            event_id: 1,
            correlation_id: 2,
            causation_id: Some(3),
            wall_ms: 1_700_000_000_000,
            clock: 9,
            lane: 4,
            depth: 2,
            event_kind: EventKind::DATA,
        });

        assert_eq!(header.position.wall_ms, 1_700_000_000_000);
        assert_eq!(header.position.sequence, 9);
        assert_eq!(header.position.lane, 4);
        assert_eq!(header.position.depth, 2);
    }
}
