// justifies: tests rely on expect/panic on unreachable failures; clippy::unwrap_used and clippy::panic are the standard harness allowances for integration tests.
#![allow(clippy::unwrap_used, clippy::panic)]
//! Durable cursor checkpoints.
//!
//! [INV-CURSOR-DURABLE] A cursor worker constructed with a `checkpoint_id`
//! persists its position atomically after every successful batch. After the
//! store is closed and reopened, a new cursor worker with the same id
//! resumes exactly from the persisted position: it sees only the events
//! that arrived after the checkpoint, never the ones it has already
//! consumed.

use batpak::coordinate::{Coordinate, Region};
use batpak::event::EventKind;
use batpak::store::delivery::cursor::{CursorCheckpoint, CursorWorkerAction, CursorWorkerConfig};
use batpak::store::{Cursor, RestartPolicy, Store, StoreConfig};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tempfile::TempDir;

const KIND: EventKind = EventKind::custom(0xA, 1);
const CHECKPOINT_ID: &str = "durable-cursor-test";

fn config(dir: &TempDir) -> StoreConfig {
    StoreConfig::new(dir.path())
        .with_enable_checkpoint(false)
        .with_enable_mmap_index(false)
        .with_sync_every_n_events(1)
}

fn wait_until(cond: impl Fn() -> bool, timeout: Duration, description: &str) {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if cond() {
            return;
        }
        std::thread::yield_now();
    }
    panic!("timed out waiting for: {description}");
}

#[test]
fn cursor_checkpoint_round_trips_through_save_and_load() {
    let dir = TempDir::new().expect("temp dir");
    let checkpoint = CursorCheckpoint {
        position: 42,
        started: true,
        process_boot_ns: None,
    };

    Cursor::save_checkpoint(dir.path(), CHECKPOINT_ID, &checkpoint).expect("save checkpoint");
    let loaded = Cursor::load_checkpoint(dir.path(), CHECKPOINT_ID)
        .expect("load checkpoint")
        .expect("checkpoint should exist");

    assert_eq!(loaded, checkpoint);
}

#[test]
fn cursor_resumes_from_checkpoint_across_reopen() {
    let dir = TempDir::new().expect("temp dir");
    let coord = Coordinate::new("entity:cursor-durable", "scope:test").expect("valid coord");

    // Phase 1: seed 100 events, spawn a cursor worker that stops after
    // processing the first 50. Capture the exact set of sequences it saw.
    let first_pass_seen: Arc<Mutex<Vec<u64>>> = {
        let first_pass_seen = Arc::new(Mutex::new(Vec::<u64>::new()));
        let store = Arc::new(Store::open(config(&dir)).expect("open store"));
        for i in 0..100u32 {
            store
                .append(&coord, KIND, &serde_json::json!({"i": i}))
                .expect("seed append");
        }

        let processed = Arc::new(AtomicU64::new(0));
        let worker = {
            let seen = Arc::clone(&first_pass_seen);
            let processed = Arc::clone(&processed);
            let mut worker_config = CursorWorkerConfig::default();
            worker_config.batch_size = 1;
            worker_config.idle_sleep = Duration::from_millis(1);
            worker_config.restart = RestartPolicy::Once;
            worker_config.checkpoint_id = Some(CHECKPOINT_ID.into());
            store
                .cursor_worker(
                    &Region::entity("entity:cursor-durable"),
                    worker_config,
                    move |batch, _store| {
                        let mut seen = seen.lock().expect("seen mutex");
                        for entry in batch {
                            seen.push(entry.global_sequence);
                        }
                        let total = processed.fetch_add(batch.len() as u64, Ordering::SeqCst)
                            + batch.len() as u64;
                        // Stop at exactly 50 events so the checkpoint records
                        // position = 49 (inclusive) / next-poll = 50.
                        if total >= 50 {
                            CursorWorkerAction::Stop
                        } else {
                            CursorWorkerAction::Continue
                        }
                    },
                )
                .expect("spawn cursor worker")
        };

        // The worker issues a durable Stop after it reaches 50 events, so
        // join() (passive) will return once that stop is observed.
        worker.join().expect("worker joined cleanly");

        let final_processed = processed.load(Ordering::SeqCst);
        assert!(
            final_processed >= 50,
            "PROPERTY: first-pass worker must process at least 50 events, got {final_processed}"
        );

        let store = match Arc::try_unwrap(store) {
            Ok(store) => store,
            Err(_) => panic!("cursor worker must release its Arc before close"),
        };
        store.close().expect("close store after first pass");
        first_pass_seen
    };

    let first_pass_set: std::collections::HashSet<u64> = first_pass_seen
        .lock()
        .expect("first_pass_seen mutex")
        .iter()
        .copied()
        .collect();

    // Phase 2: reopen, append 50 more events, spawn a NEW worker with the
    // same checkpoint_id. It must see ONLY the new events.
    {
        let store = Arc::new(Store::open(config(&dir)).expect("reopen store"));
        for i in 100..150u32 {
            store
                .append(&coord, KIND, &serde_json::json!({"i": i}))
                .expect("post-reopen append");
        }

        let second_pass_seen: Arc<Mutex<Vec<u64>>> = Arc::new(Mutex::new(Vec::new()));
        let processed = Arc::new(AtomicU64::new(0));

        let worker = {
            let seen = Arc::clone(&second_pass_seen);
            let processed = Arc::clone(&processed);
            let mut worker_config = CursorWorkerConfig::default();
            worker_config.batch_size = 8;
            worker_config.idle_sleep = Duration::from_millis(1);
            worker_config.restart = RestartPolicy::Once;
            worker_config.checkpoint_id = Some(CHECKPOINT_ID.into());
            store
                .cursor_worker(
                    &Region::entity("entity:cursor-durable"),
                    worker_config,
                    move |batch, _store| {
                        let mut seen = seen.lock().expect("seen mutex");
                        for entry in batch {
                            seen.push(entry.global_sequence);
                        }
                        processed.fetch_add(batch.len() as u64, Ordering::SeqCst);
                        CursorWorkerAction::Continue
                    },
                )
                .expect("spawn second-pass cursor worker")
        };

        // Wait until the second-pass worker has consumed the remaining
        // new events. The worker keeps polling; we stop it externally once
        // the observable progress counter reaches the expected count.
        // The durable checkpoint recorded position ~50, so the second pass
        // must cover sequences 50..150 — 100 events total.
        let processed_for_wait = Arc::clone(&processed);
        wait_until(
            || processed_for_wait.load(Ordering::SeqCst) >= 100,
            Duration::from_secs(5),
            "second-pass cursor to process the 100 post-checkpoint events",
        );

        worker.stop_and_join().expect("stop second-pass worker");

        let second_pass = second_pass_seen.lock().expect("second_pass mutex").clone();

        // Every sequence the second pass observed must be strictly greater
        // than any sequence the first pass observed. If the two sets
        // overlap, the durable checkpoint was not honoured.
        let overlap: Vec<u64> = second_pass
            .iter()
            .filter(|seq| first_pass_set.contains(*seq))
            .copied()
            .collect();
        assert!(
            overlap.is_empty(),
            "PROPERTY: a cursor resumed from its durable checkpoint must never re-deliver \
             events the previous run already acked. Overlap: {overlap:?}"
        );

        // The second pass must cover exactly the post-checkpoint tail
        // (the remaining 50 originals + 50 new = 100 events). The first
        // pass covered up to 50; the tail is 100 events.
        assert_eq!(
            second_pass.len(),
            100,
            "PROPERTY: second-pass worker must cover the 100 post-checkpoint events; got {}",
            second_pass.len()
        );

        let store = match Arc::try_unwrap(store) {
            Ok(store) => store,
            Err(_) => panic!("cursor worker must release its Arc before close"),
        };
        store.close().expect("close store after second pass");
    }
}
