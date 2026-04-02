#![allow(clippy::unwrap_used)]
//! Deterministic concurrency proofs using loom.
//! [SPEC:tests/deterministic_concurrency.rs]

use loom::sync::{Arc, Mutex};
use loom::thread;

fn model_idempotent_append(committed: &Mutex<bool>, commit_count: &Mutex<u64>) {
    let mut committed_guard = committed.lock().unwrap();
    if !*committed_guard {
        *committed_guard = true;
        drop(committed_guard);

        let mut count_guard = commit_count.lock().unwrap();
        *count_guard += 1;
    }
}

fn model_compare_and_append(sequence: &Mutex<u32>, success_count: &Mutex<u32>, expected: u32) {
    let mut sequence_guard = sequence.lock().unwrap();
    if *sequence_guard == expected {
        *sequence_guard += 1;
        drop(sequence_guard);

        let mut success_guard = success_count.lock().unwrap();
        *success_guard += 1;
    }
}

fn model_bounded_restart(
    restart_count: &Mutex<u32>,
    successful_restarts: &Mutex<u32>,
    max_restarts: u32,
) {
    let mut restart_guard = restart_count.lock().unwrap();
    if *restart_guard < max_restarts {
        *restart_guard += 1;
        drop(restart_guard);

        let mut success_guard = successful_restarts.lock().unwrap();
        *success_guard += 1;
    }
}

fn model_single_compactor(compacting: &Mutex<bool>, winners: &Mutex<u32>) {
    let mut compacting_guard = compacting.lock().unwrap();
    if !*compacting_guard {
        *compacting_guard = true;
        drop(compacting_guard);

        let mut winners_guard = winners.lock().unwrap();
        *winners_guard += 1;
    }
}

#[test]
fn loom_idempotency_single_winner_under_race() {
    loom::model(|| {
        let committed = Arc::new(Mutex::new(false));
        let commit_count = Arc::new(Mutex::new(0_u64));

        let mut handles = Vec::new();
        for _ in 0..2 {
            let committed = Arc::clone(&committed);
            let commit_count = Arc::clone(&commit_count);
            handles.push(thread::spawn(move || {
                model_idempotent_append(&committed, &commit_count);
            }));
        }

        for handle in handles {
            handle.join().unwrap();
        }

        assert!(
            *committed.lock().unwrap(),
            "PROPERTY: one racing append must commit the idempotent key."
        );
        assert_eq!(
            *commit_count.lock().unwrap(),
            1,
            "PROPERTY: racing idempotent appends must linearize to a single committed write."
        );
    });
}

#[test]
fn loom_cas_only_one_writer_can_claim_sequence() {
    loom::model(|| {
        let sequence = Arc::new(Mutex::new(0_u32));
        let success_count = Arc::new(Mutex::new(0_u32));

        let mut handles = Vec::new();
        for _ in 0..2 {
            let sequence = Arc::clone(&sequence);
            let success_count = Arc::clone(&success_count);
            handles.push(thread::spawn(move || {
                model_compare_and_append(&sequence, &success_count, 0);
            }));
        }

        for handle in handles {
            handle.join().unwrap();
        }

        assert_eq!(
            *success_count.lock().unwrap(),
            1,
            "PROPERTY: two racing CAS appends with expected_sequence=0 must have exactly one winner."
        );
        assert_eq!(
            *sequence.lock().unwrap(),
            1,
            "PROPERTY: the claimed sequence must advance exactly once after the race."
        );
    });
}

#[test]
fn loom_bounded_restart_allows_only_configured_number_of_recoveries() {
    loom::model(|| {
        let restart_count = Arc::new(Mutex::new(0_u32));
        let successful_restarts = Arc::new(Mutex::new(0_u32));

        let mut handles = Vec::new();
        for _ in 0..2 {
            let restart_count = Arc::clone(&restart_count);
            let successful_restarts = Arc::clone(&successful_restarts);
            handles.push(thread::spawn(move || {
                model_bounded_restart(&restart_count, &successful_restarts, 1);
            }));
        }

        for handle in handles {
            handle.join().unwrap();
        }

        assert_eq!(
            *successful_restarts.lock().unwrap(),
            1,
            "PROPERTY: a bounded restart policy with max_restarts=1 must admit exactly one recovery under race."
        );
        assert_eq!(
            *restart_count.lock().unwrap(),
            1,
            "PROPERTY: restart bookkeeping must stop once the configured limit is exhausted."
        );
    });
}

#[test]
fn loom_compaction_has_single_exclusive_owner() {
    loom::model(|| {
        let compacting = Arc::new(Mutex::new(false));
        let winners = Arc::new(Mutex::new(0_u32));

        let mut handles = Vec::new();
        for _ in 0..2 {
            let compacting = Arc::clone(&compacting);
            let winners = Arc::clone(&winners);
            handles.push(thread::spawn(move || {
                model_single_compactor(&compacting, &winners);
            }));
        }

        for handle in handles {
            handle.join().unwrap();
        }

        assert_eq!(
            *winners.lock().unwrap(),
            1,
            "PROPERTY: only one compaction claimant may own the exclusive window at a time."
        );
    });
}
