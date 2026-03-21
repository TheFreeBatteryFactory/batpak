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

// ---- Multi-dimension performance gates ----
// The benchmark tells you WHAT needs improving, not just pass/fail.

/// Write throughput gate: events/sec must meet minimum.
struct WriteThroughputGate {
    min_events_per_sec: f64,
}

impl Gate<PerfContext> for WriteThroughputGate {
    fn name(&self) -> &'static str { "write_throughput" }

    fn evaluate(&self, ctx: &PerfContext) -> Result<(), Denial> {
        if ctx.events_per_sec >= self.min_events_per_sec {
            Ok(())
        } else {
            Err(Denial::new(
                "write_throughput",
                format!(
                    "Write throughput {:.0} events/sec < minimum {:.0}. \
                     Investigate: src/store/writer.rs handle_append (10-step commit), \
                     src/store/segment.rs write_frame, CRC overhead.",
                    ctx.events_per_sec, self.min_events_per_sec
                ),
            ).with_context("events_per_sec", format!("{:.0}", ctx.events_per_sec))
             .with_context("min_required", format!("{:.0}", self.min_events_per_sec)))
        }
    }
}

/// Query latency gate: microseconds per query must meet maximum.
struct QueryLatencyGate {
    max_us_per_query: f64,
}

impl Gate<PerfContext> for QueryLatencyGate {
    fn name(&self) -> &'static str { "query_latency" }

    fn evaluate(&self, ctx: &PerfContext) -> Result<(), Denial> {
        if ctx.query_us <= self.max_us_per_query {
            Ok(())
        } else {
            Err(Denial::new(
                "query_latency",
                format!(
                    "Query latency {:.1}µs > max {:.1}µs. \
                     Investigate: src/store/index.rs query() DashMap scan, \
                     Region::matches_event hot path.",
                    ctx.query_us, self.max_us_per_query
                ),
            ).with_context("query_us", format!("{:.1}", ctx.query_us))
             .with_context("max_us", format!("{:.1}", self.max_us_per_query)))
        }
    }
}

/// Projection gate: replay time must be bounded.
struct ProjectionGate {
    max_ms: f64,
}

impl Gate<PerfContext> for ProjectionGate {
    fn name(&self) -> &'static str { "projection_replay" }

    fn evaluate(&self, ctx: &PerfContext) -> Result<(), Denial> {
        if ctx.projection_ms <= self.max_ms {
            Ok(())
        } else {
            Err(Denial::new(
                "projection_replay",
                format!(
                    "Projection replay {:.1}ms > max {:.1}ms for {} events. \
                     Investigate: src/store/mod.rs project(), \
                     src/store/reader.rs read_entry deserialization.",
                    ctx.projection_ms, self.max_ms, ctx.event_count
                ),
            ).with_context("projection_ms", format!("{:.1}", ctx.projection_ms))
             .with_context("max_ms", format!("{:.1}", self.max_ms))
             .with_context("event_count", ctx.event_count.to_string()))
        }
    }
}

struct PerfContext {
    event_count: u64,
    events_per_sec: f64,
    query_us: f64,
    projection_ms: f64,
}

/// The multi-gate self-benchmark. Uses evaluate_all() to collect ALL denials,
/// not fail-fast — so it reports EVERYTHING that needs improvement in one pass.
#[derive(Default, Debug)]
struct BenchCounter {
    count: u64,
}

impl EventSourced<serde_json::Value> for BenchCounter {
    fn from_events(events: &[Event<serde_json::Value>]) -> Option<Self> {
        if events.is_empty() { return None; }
        let mut s = Self::default();
        for e in events { s.apply_event(e); }
        Some(s)
    }
    fn apply_event(&mut self, _event: &Event<serde_json::Value>) {
        self.count += 1;
    }
    fn relevant_event_kinds() -> &'static [EventKind] {
        static KINDS: [EventKind; 1] = [EventKind::custom(0xF, 1)];
        &KINDS
    }
}

#[test]
fn multi_gate_performance_feedback() {
    let dir = TempDir::new().expect("temp dir");
    let config = StoreConfig {
        data_dir: dir.path().to_path_buf(),
        ..StoreConfig::default()
    };
    let store = Store::open(config).expect("open");
    let coord = Coordinate::new("perf:entity", "perf:scope").expect("valid coord");
    let kind = EventKind::custom(0xF, 1);
    let n = 1_000u64;

    // Measure write throughput
    let write_start = Instant::now();
    for i in 0..n {
        store.append(&coord, kind, &serde_json::json!({"i": i})).expect("append");
    }
    let write_elapsed = write_start.elapsed();
    let events_per_sec = n as f64 / write_elapsed.as_secs_f64();

    // Measure query latency
    let query_iters = 100u64;
    let query_start = Instant::now();
    let region = Region::entity("perf:entity");
    for _ in 0..query_iters {
        let _ = store.query(&region);
    }
    let query_elapsed = query_start.elapsed();
    let query_us = query_elapsed.as_micros() as f64 / query_iters as f64;

    // Measure projection replay
    let proj_start = Instant::now();
    let _: Option<BenchCounter> = store
        .project("perf:entity", free_batteries::store::Freshness::Consistent)
        .expect("project");
    let projection_ms = proj_start.elapsed().as_secs_f64() * 1000.0;

    let ctx = PerfContext {
        event_count: n,
        events_per_sec,
        query_us,
        projection_ms,
    };

    // Build gate set with thresholds (generous for CI, tighten for prod)
    let mut gates = GateSet::new();
    gates.push(WriteThroughputGate { min_events_per_sec: 1_000.0 });  // 1K/sec minimum
    gates.push(QueryLatencyGate { max_us_per_query: 50_000.0 });       // 50ms max
    gates.push(ProjectionGate { max_ms: 5_000.0 });                    // 5s max for 1K events

    // evaluate_all: collect ALL denials, don't stop at first
    let denials = gates.evaluate_all(&ctx);

    // Report — this IS the benchmark feedback
    eprintln!("\n  SELF-BENCHMARK REPORT ({n} events):");
    eprintln!("    Write throughput:  {events_per_sec:.0} events/sec");
    eprintln!("    Query latency:     {query_us:.1} µs/query");
    eprintln!("    Projection replay: {projection_ms:.1} ms");

    if denials.is_empty() {
        eprintln!("    Result: ALL GATES PASSED");
    } else {
        eprintln!("    Result: {} GATES FAILED:", denials.len());
        for d in &denials {
            eprintln!("      [{gate}] {msg}", gate = d.gate, msg = d.message);
            for (k, v) in &d.context {
                eprintln!("        {k} = {v}");
            }
        }
        panic!(
            "SELF-BENCHMARK FAILED: {} performance gate(s) denied.\n\
             The denials above tell you exactly where to look.\n\
             This is the library using its own Gate system to enforce its own quality.",
            denials.len()
        );
    }

    store.close().expect("close");
}

/// Verify multi-gate reports ALL failures, not just the first.
#[test]
fn multi_gate_collects_all_denials() {
    let ctx = PerfContext {
        event_count: 1000,
        events_per_sec: 1.0,       // way too slow
        query_us: 999_999.0,       // way too slow
        projection_ms: 999_999.0,  // way too slow
    };

    let mut gates = GateSet::new();
    gates.push(WriteThroughputGate { min_events_per_sec: 1_000.0 });
    gates.push(QueryLatencyGate { max_us_per_query: 50_000.0 });
    gates.push(ProjectionGate { max_ms: 5_000.0 });

    let denials = gates.evaluate_all(&ctx);
    assert_eq!(denials.len(), 3,
        "evaluate_all should report ALL 3 failures, not fail-fast. Got {}.",
        denials.len());

    // Verify each denial points to the right gate and has actionable context
    assert_eq!(denials[0].gate, "write_throughput");
    assert_eq!(denials[1].gate, "query_latency");
    assert_eq!(denials[2].gate, "projection_replay");

    // Verify context has the "investigate" pointers
    assert!(denials[0].message.contains("writer.rs"),
        "Denial should point to file to investigate");
    assert!(denials[1].message.contains("index.rs"),
        "Denial should point to file to investigate");
    assert!(denials[2].message.contains("reader.rs"),
        "Denial should point to file to investigate");
}
