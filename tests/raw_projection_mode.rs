#![allow(clippy::panic)]

use std::sync::Arc;

use batpak::prelude::*;
use batpak::store::{Freshness, Store, StoreConfig};
use serde::{Deserialize, Serialize};
use tempfile::TempDir;

const KIND: EventKind = EventKind::custom(0xF, 0x31);

#[derive(Clone, Debug, Serialize, Deserialize)]
struct CounterDelta {
    amount: i64,
    label: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
struct ValueCounter {
    value: i64,
    seen: u32,
}

impl EventSourced for ValueCounter {
    type Input = ValueInput;

    fn from_events(events: &[Event<serde_json::Value>]) -> Option<Self> {
        if events.is_empty() {
            return None;
        }
        let mut state = Self::default();
        for event in events {
            state.apply_event(event);
        }
        Some(state)
    }

    fn apply_event(&mut self, event: &Event<serde_json::Value>) {
        if let Ok(delta) = serde_json::from_value::<CounterDelta>(event.payload.clone()) {
            self.value += delta.amount;
            self.seen += 1;
        }
    }

    fn relevant_event_kinds() -> &'static [EventKind] {
        &[KIND]
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
struct RawCounter {
    value: i64,
    seen: u32,
}

impl EventSourced for RawCounter {
    type Input = RawMsgpackInput;

    fn from_events(events: &[Event<Vec<u8>>]) -> Option<Self> {
        if events.is_empty() {
            return None;
        }
        let mut state = Self::default();
        for event in events {
            state.apply_event(event);
        }
        Some(state)
    }

    fn apply_event(&mut self, event: &Event<Vec<u8>>) {
        if let Ok(delta) = rmp_serde::from_slice::<CounterDelta>(&event.payload) {
            self.value += delta.amount;
            self.seen += 1;
        }
    }

    fn relevant_event_kinds() -> &'static [EventKind] {
        &[KIND]
    }
}

// Compile-time assertions: replay mode selection and payload type aliases work correctly.
const _: () = assert!(matches!(
    <RawMsgpackInput as ProjectionInput>::MODE,
    ProjectionMode::RawMsgpack
));

// ProjectionPayload and ProjectionEvent resolve to the correct concrete types.
fn _assert_projection_type_aliases() {
    fn _is_vec_u8(_: ProjectionPayload<RawCounter>) {}
    fn _is_value(_: ProjectionPayload<ValueCounter>) {}
    fn _is_raw_event(_: ProjectionEvent<RawCounter>) {}
    fn _is_value_event(_: ProjectionEvent<ValueCounter>) {}
}

fn seeded_store() -> (Arc<Store>, TempDir) {
    let dir = TempDir::new().expect("temp dir");
    let store = Arc::new(Store::open(StoreConfig::new(dir.path())).expect("open"));
    let coord = Coordinate::new("entity:raw-proj", "scope:test").expect("coord");
    for (amount, label) in [(3, "a"), (-1, "b"), (7, "c"), (2, "d")] {
        store
            .append(
                &coord,
                KIND,
                &CounterDelta {
                    amount,
                    label: label.to_owned(),
                },
            )
            .expect("append");
    }
    (store, dir)
}

#[test]
fn raw_projection_matches_value_projection_live_and_reopen() {
    let (store, dir) = seeded_store();

    let value_live: Option<ValueCounter> = store
        .project("entity:raw-proj", &Freshness::Consistent)
        .expect("value project");
    let raw_live: Option<RawCounter> = store
        .project("entity:raw-proj", &Freshness::Consistent)
        .expect("raw project");
    assert_eq!(
        raw_live.as_ref().map(|state| (state.value, state.seen)),
        value_live.as_ref().map(|state| (state.value, state.seen)),
        "raw-mode and value-mode projections must converge on the same state live"
    );

    let store = match Arc::try_unwrap(store) {
        Ok(store) => store,
        Err(_) => {
            panic!("PROPERTY: raw projection test should release all Arc clones before close")
        }
    };
    store.close().expect("close");

    let reopened = Store::open(StoreConfig::new(dir.path())).expect("reopen");
    let value_reopen: Option<ValueCounter> = reopened
        .project("entity:raw-proj", &Freshness::Consistent)
        .expect("value project after reopen");
    let raw_reopen: Option<RawCounter> = reopened
        .project("entity:raw-proj", &Freshness::Consistent)
        .expect("raw project after reopen");
    assert_eq!(
        raw_reopen.as_ref().map(|state| (state.value, state.seen)),
        value_reopen.as_ref().map(|state| (state.value, state.seen)),
        "raw-mode and value-mode projections must converge after cold start too"
    );
    reopened.close().expect("close reopened");
}

#[test]
fn raw_watch_projection_emits_updated_state() {
    let (store, _dir) = seeded_store();
    let mut watcher =
        Arc::clone(&store).watch_projection::<RawCounter>("entity:raw-proj", Freshness::Consistent);
    let coord = Coordinate::new("entity:raw-proj", "scope:test").expect("coord");

    store
        .append(
            &coord,
            KIND,
            &CounterDelta {
                amount: 5,
                label: "watch".to_owned(),
            },
        )
        .expect("append watch event");

    let update = watcher
        .recv()
        .expect("watch projection recv")
        .expect("watch projection state");
    assert_eq!(update.value, 16);
    assert_eq!(update.seen, 5);

    drop(watcher);
    let store = match Arc::try_unwrap(store) {
        Ok(store) => store,
        Err(_) => panic!("PROPERTY: raw watch test should release all Arc clones before close"),
    };
    store.close().expect("close");
}
