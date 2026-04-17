use crate::coordinate::Coordinate;
use crate::event::{Event, EventKind};

mod sealed {
    pub trait Sealed {}
}

/// Internal-friendly marker describing which replay lane the store should use
/// for a projection. This stays tiny and data-oriented on purpose.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReplayLane {
    /// Decode projection payloads into `serde_json::Value`.
    Value,
    /// Keep projection payloads as raw MessagePack bytes.
    RawMsgpack,
}

/// Marker trait selecting how projection replay decodes event payloads.
///
/// The store owns the concrete replay pipeline for each input mode. Projection
/// implementations pick the mode via their associated `Input` type and then
/// operate over `Event<<Self::Input as ProjectionInput>::Payload>`.
pub trait ProjectionInput: sealed::Sealed + Send + Sync + 'static {
    /// Payload type produced by the store for this replay mode.
    type Payload: Clone + Send + Sync + 'static;
    /// Replay lane selected for this projection input type.
    const MODE: ReplayLane;
}

/// Default projection replay mode: payloads are decoded into `serde_json::Value`.
#[derive(Clone, Copy, Debug, Default)]
pub struct JsonValueInput;

impl sealed::Sealed for JsonValueInput {}

impl ProjectionInput for JsonValueInput {
    type Payload = serde_json::Value;
    const MODE: ReplayLane = ReplayLane::Value;
}

/// Raw replay mode: payloads remain in their original MessagePack bytes.
#[derive(Clone, Copy, Debug, Default)]
pub struct RawMsgpackInput;

impl sealed::Sealed for RawMsgpackInput {}

impl ProjectionInput for RawMsgpackInput {
    type Payload = Vec<u8>;
    const MODE: ReplayLane = ReplayLane::RawMsgpack;
}

/// Convenience alias for the payload shape used by a projection type.
pub type ProjectionPayload<T> = <<T as EventSourced>::Input as ProjectionInput>::Payload;

/// Convenience alias for the event shape used by a projection type.
pub type ProjectionEvent<T> = Event<ProjectionPayload<T>>;

/// `EventSourced`: backward-looking fold. Replay events to reconstruct state.
///
/// The associated `Input` selects the replay decode lane. The default and
/// most ergonomic choice is [`JsonValueInput`], which preserves the current
/// `serde_json::Value` projection behavior. Implement [`RawMsgpackInput`] only
/// when the projection benefits from operating directly on raw MessagePack
/// payload bytes.
pub trait EventSourced: Sized {
    /// Replay decode mode used for this projection.
    type Input: ProjectionInput;

    /// Reconstructs state by folding over a slice of events.
    ///
    /// `None` means the stream is valid but produces no state.
    fn from_events(events: &[ProjectionEvent<Self>]) -> Option<Self>;
    /// Advances state by incorporating a single event.
    fn apply_event(&mut self, event: &ProjectionEvent<Self>);
    /// Returns the event kinds this type cares about, used to filter store queries.
    /// The store uses this as a hard filter: only matching events are loaded from disk
    /// and passed to `from_events()`. Empty slice means "no filter — replay all events."
    fn relevant_event_kinds() -> &'static [EventKind];

    /// Schema version for projection cache isolation. Increment this when the
    /// serialized shape of this type changes in a breaking way. Default: 0.
    /// Different versions get separate cache keys — old cached projections
    /// are not served to new code.
    fn schema_version() -> u64 {
        0
    }

    /// Returns `true` if this type supports incremental application: loading a
    /// cached state at a watermark and calling `apply_event()` only for events
    /// newer than that watermark, instead of replaying from scratch.
    ///
    /// Opt-in — `false` by default.
    ///
    /// Only set to `true` if `from_events()` is a pure fold over
    /// `apply_event()` and `apply_event()` is infallible for every event the
    /// projection accepts. The incremental replay path has no separate error
    /// channel; violating this contract makes cached incremental replay diverge
    /// from full replay.
    fn supports_incremental_apply() -> bool {
        false
    }
}

/// `Reactive<P>`: forward-looking counterpart. See event → maybe emit derived events.
/// Products compose: subscribe + react + append (7 lines of glue).
///
/// # Manual Glue Pattern
/// ```no_run
/// # use batpak::prelude::*;
/// # use batpak::event::sourcing::Reactive;
/// # struct MyReactor;
/// # impl Reactive<serde_json::Value> for MyReactor {
/// #     fn react(&self, _event: &Event<serde_json::Value>) -> Vec<(Coordinate, EventKind, serde_json::Value)> { vec![] }
/// # }
/// # fn example(store: &Store, reactor: &MyReactor) {
/// let region = Region::entity("order:*");
/// let sub = store.subscribe_lossy(&region);
/// while let Some(notif) = sub.recv() {
///     let stored = store.get(notif.event_id).unwrap();
///     for (coord, kind, payload) in reactor.react(&stored.event) {
///         store.append_reaction(&coord, kind, &payload, notif.correlation_id, notif.event_id).unwrap();
///     }
/// }
/// # }
/// ```
///
/// For convenience, use [`Store::react_loop`](crate::store::Store::react_loop) which
/// spawns a thread running this pattern automatically.
pub trait Reactive<P> {
    /// Inspects an incoming event and returns zero or more derived events to append.
    fn react(&self, event: &Event<P>) -> Vec<(Coordinate, EventKind, P)>;
}
