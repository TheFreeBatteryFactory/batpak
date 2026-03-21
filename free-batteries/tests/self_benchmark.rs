//! Self-benchmark test: the library dogfoods its own Gate system.
//! A Gate validates that cold-start time for N events stays under a threshold.
//! [SPEC:tests/self_benchmark.rs]
//!
//! This IS the "free battery factory" philosophy: the same Gate/Pipeline system
//! that products use to enforce business rules, the library uses to enforce
//! its own performance characteristics. If gates work, this test passes.
//! If this test passes, gates work. Quadratic feedback.

use free_batteries::prelude::*;
use free_batteries::store::{Store, StoreConfig};
use tempfile::TempDir;
use std::time::Instant;

/// A Gate that checks cold-start performance.
/// This is not a unit test — it's the library testing itself with its own tools.
struct ColdStartGate {
    max_ms: u128,
}

impl Gate<ColdStartContext> for ColdStartGate {
    fn name(&self) -> &'static str { "cold_start_performance" }

    fn evaluate(&self, ctx: &ColdStartContext) -> Result<(), Denial> {
        if ctx.cold_start_ms <= self.max_ms {
            Ok(())
        } else {
            Err(Denial::new(
                "cold_start_performance",
                format!(
                    "Cold start took {}ms for {} events (max: {}ms). \
                     Investigate: src/store/mod.rs Store::open cold start scan, \
                     src/store/reader.rs scan_segment.",
                    ctx.cold_start_ms, ctx.event_count, self.max_ms
                ),
            ).with_context("event_count", ctx.event_count.to_string())
             .with_context("cold_start_ms", ctx.cold_start_ms.to_string())
             .with_context("max_ms", self.max_ms.to_string()))
        }
    }
}

struct ColdStartContext {
    cold_start_ms: u128,
    event_count: u64,
}

#[test]
fn cold_start_1k_events_under_threshold() {
    let dir = TempDir::new().expect("create temp dir");
    let kind = EventKind::custom(0xF, 1);
    let payload = serde_json::json!({"x": 1});

    // Populate
    {
        let config = StoreConfig {
            data_dir: dir.path().to_path_buf(),
            ..StoreConfig::default()
        };
        let store = Store::open(config).expect("open store");
        let coord = Coordinate::new("bench:entity", "bench:scope").expect("valid coord");
        for _ in 0..1_000 {
            store.append(&coord, kind, &payload).expect("append");
        }
        store.sync().expect("sync");
        store.close().expect("close");
    }

    // Measure cold start
    let start = Instant::now();
    let config = StoreConfig {
        data_dir: dir.path().to_path_buf(),
        ..StoreConfig::default()
    };
    let store = Store::open(config).expect("cold start");
    let cold_start_ms = start.elapsed().as_millis();

    // Dogfood: use our own Gate system to validate performance
    let mut gates = GateSet::new();
    gates.push(ColdStartGate { max_ms: 2000 }); // 2s threshold for 1K events (generous for CI)

    let ctx = ColdStartContext {
        cold_start_ms,
        event_count: 1_000,
    };

    let proposal = Proposal::new(cold_start_ms);
    let result = gates.evaluate(&ctx, proposal);

    match result {
        Ok(receipt) => {
            let (ms, gate_names) = receipt.into_parts();
            assert_eq!(gate_names, vec!["cold_start_performance"],
                "Gate should report its name in the receipt.");
            eprintln!("SELF-BENCHMARK: cold start for 1K events: {}ms (passed {})",
                ms, gate_names.join(", "));
        }
        Err(denial) => {
            panic!("SELF-BENCHMARK FAILED: {}\n\
                    The library's own Gate system detected a performance regression.\n\
                    Context: {:?}",
                denial, denial.context);
        }
    }

    store.sync().expect("sync");
}

/// Verify the Gate system correctly rejects slow cold starts.
/// This tests that the dogfood mechanism itself works — it would catch
/// a broken Gate that always passes.
#[test]
fn cold_start_gate_rejects_slow() {
    let mut gates = GateSet::new();
    gates.push(ColdStartGate { max_ms: 1 }); // impossibly tight

    let ctx = ColdStartContext {
        cold_start_ms: 100, // simulated slow cold start
        event_count: 1_000,
    };

    let proposal = Proposal::new(100u128);
    let result = gates.evaluate(&ctx, proposal);
    assert!(result.is_err(),
        "Gate should reject cold start that exceeds threshold. \
         If this test passes incorrectly, the self-benchmark dogfood is broken.");
}
