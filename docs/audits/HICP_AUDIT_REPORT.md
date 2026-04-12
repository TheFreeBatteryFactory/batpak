# Audit Report: batpak

> **Note (2026-04-11):** This audit was conducted against the pre-0.3.0
> codebase. Sections referencing `RedbCache`, `LmdbCache`, the `redb`/`lmdb`
> Cargo features, `entity_locks`, or flat `StoreConfig` fields are
> historical — see CHANGELOG.md for the 0.3.0 changes.

Evidence run on 2026-03-30:
- `cargo test --all-features`: pass
- `cargo check --no-default-features`: pass
- `cargo fmt --check`: pass
- `cargo bench --no-run --all-features`: pass
- `cargo clippy --all-features --all-targets -- -D warnings`: fail at `tests/subscription_ops.rs:434` (`panic!`)
- `cargo deny check`: fail parsing `batpak/deny.toml` on `unmaintained = "warn"`

Evidence-only inputs used but not scored: `batpak/Cargo.lock`, `batpak/tests/fuzz_targets.proptest-regressions`, `batpak/tests/golden/*.hex`, `batpak/tests/ui/*.stderr`, `batpak/LICENSE-*`, `.claude/settings.local.json`.

## Crate: batpak

### batpak/.gitignore
Applicable Parameters: Build-Config
Score: 72/100
Notes: Basic hygiene is present, but this file is thin and does not help determinism or traceability beyond excluding local artifacts.
Named Offenses / Forbidden Remedies: None confirmed.

### batpak/.cargo/config.toml
Applicable Parameters: Build-Config
Score: 48/100
Notes: This hardcodes a developer-specific Windows MSVC linker path, which weakens Environment Determinism and makes the build sensitive to unmanaged host state.
Named Offenses / Forbidden Remedies: Structural finding: non-hermetic environment coupling.

### batpak/.config/nextest.toml
Applicable Parameters: Build-Config
Score: 84/100
Notes: Strong deterministic test-runner settings and JUnit output support self-accusation; limited only by being execution policy rather than architectural proof.
Named Offenses / Forbidden Remedies: None confirmed.

### batpak/.github/workflows/ci.yml
Applicable Parameters: Build-Config
Score: 74/100
Notes: CI covers checks, tests, benches, docs, fuzz/chaos, and coverage, but the clippy job omits `--all-targets`, which let the current `tests/subscription_ops.rs` lint failure escape the declared gate.
Named Offenses / Forbidden Remedies: Coverage Mirage risk on lint surface.

### batpak/ARCHITECTURE.md
Applicable Parameters: Spec-Docs
Score: 78/100
Notes: Useful narrative architecture guide with explicit invariants and build-time checks, but it remains prose rather than machine-checked traceability.
Named Offenses / Forbidden Remedies: None confirmed.

### batpak/build.rs
Applicable Parameters: Build-Config
Score: 87/100
Notes: This is the strongest self-accusation surface in the crate: it enforces no-stub, no-tokio, allow-justification, config wiring, and public-item test linkage. Score is reduced because it relies on string scanning and `panic!`-driven enforcement rather than richer structural analysis.
Named Offenses / Forbidden Remedies: None confirmed; enforcement uses intentional `panic!` in build context, not production paths.

### batpak/Cargo.toml
Applicable Parameters: Build-Config
Score: 89/100
Notes: Strong dependency pinning, feature isolation, clippy policy, and benchmark wiring. The main gap is that dependency governance is not fully aligned with the local `cargo deny` behavior.
Named Offenses / Forbidden Remedies: None confirmed.

### batpak/CHANGELOG.md
Applicable Parameters: Spec-Docs
Score: 68/100
Notes: Changelog exists, which helps decision capture, but it is too thin to support bidirectional traceability or freeze-conflict history on its own.
Named Offenses / Forbidden Remedies: None confirmed.

### batpak/clippy.toml
Applicable Parameters: Build-Config
Score: 84/100
Notes: Strong banned-method policy and complexity thresholds directly accuse common AI failure modes. Score is reduced because the active CI and local wrapper commands do not consistently apply this policy to all targets.
Named Offenses / Forbidden Remedies: None confirmed.

### batpak/deny.toml
Applicable Parameters: Build-Config
Score: 42/100
Notes: The intent is good, but the current config does not parse under the local `cargo deny check`, so the supply-chain gate is not currently trustworthy as executed evidence.
Named Offenses / Forbidden Remedies: Structural finding: broken dependency-audit gate.

### batpak/docs/superpowers/plans/2026-03-30-test-bench-reorganization.md
Applicable Parameters: Spec-Docs
Score: 34/100
Notes: This internal plan is materially stale: it still references `tests/self_benchmark.rs`, `tests/quiet_stragglers.rs`, outdated TODOs, and old test counts, so it now creates traceability drift instead of reducing it.
Named Offenses / Forbidden Remedies: Bidirectional traceability failure.

### batpak/justfile
Applicable Parameters: Build-Config
Score: 70/100
Notes: Useful operator shortcuts exist, but `ci` and `clip` do not run clippy against all targets and do not include the deny gate, so the local workflow under-enforces the declared standard.
Named Offenses / Forbidden Remedies: Coverage Mirage risk on local gate surface.

### batpak/README.md
Applicable Parameters: Spec-Docs
Score: 80/100
Notes: Clear public summary of invariants, architecture, and project layout. It is informative, but not enough to satisfy bidirectional requirement-to-artifact traceability by itself.
Named Offenses / Forbidden Remedies: None confirmed.

### batpak/rust-toolchain.toml
Applicable Parameters: Build-Config
Score: 88/100
Notes: Strong toolchain pinning for formatter, clippy, and rust-src improves reproducibility across environments.
Named Offenses / Forbidden Remedies: None confirmed.

### batpak/scripts/bench-report
Applicable Parameters: DevOps-Scripts
Score: 78/100
Notes: This strengthens benchmark feedback loops and baseline comparison, but it remains advisory rather than an invariant-enforcing gate.
Named Offenses / Forbidden Remedies: None confirmed.

### batpak/scripts/coverage-feedback
Applicable Parameters: DevOps-Scripts
Score: 80/100
Notes: Good coverage feedback support and CI threshold integration. Score is reduced because coverage remains line-oriented and therefore only partially addresses Coverage Mirage.
Named Offenses / Forbidden Remedies: Coverage Mirage risk remains partially unclosed.

### batpak/TUNING.md
Applicable Parameters: Spec-Docs
Score: 76/100
Notes: Helpful operational guidance for Store configuration and tradeoffs. It explains behavior but does not fully tie settings back to invariant proofs or rollout evidence.
Named Offenses / Forbidden Remedies: None confirmed.

### batpak/src/lib.rs
Applicable Parameters: Core-Source
Score: 86/100
Notes: Clean dependency-ordered module surface and compile-time feature guards align with architecture freeze. Score reduced for `unexpected_cfgs` allowances and prose-only dependency-order enforcement.
Named Offenses / Forbidden Remedies: None confirmed.

### batpak/src/prelude.rs
Applicable Parameters: Core-Source
Score: 84/100
Notes: Thin, honest re-export layer with no business logic. Score reduced because broad public aggregation increases surface area and makes visibility creep easier to miss.
Named Offenses / Forbidden Remedies: None confirmed.

### batpak/src/wire.rs
Applicable Parameters: Core-Source
Score: 91/100
Notes: Strong semantic serialization helpers with no stub patterns and clear purpose; golden and fuzz tests make this a well-proved low-level module.
Named Offenses / Forbidden Remedies: None confirmed.

### batpak/src/coordinate/mod.rs
Applicable Parameters: Core-Source
Score: 88/100
Notes: Semantic types (`Coordinate`, `Region`, `KindFilter`) preserve meaning at boundaries and are well-covered by API tests. Minor reduction for prose-only semantic contracts.
Named Offenses / Forbidden Remedies: None confirmed.

### batpak/src/coordinate/position.rs
Applicable Parameters: Core-Source
Score: 90/100
Notes: Compact, deterministic causal-position logic with strong direct test coverage and no silence/stub signals.
Named Offenses / Forbidden Remedies: None confirmed.

### batpak/src/event/hash.rs
Applicable Parameters: Core-Source
Score: 91/100
Notes: Small, focused hashing module with direct tamper-detection tests and no fake-success patterns.
Named Offenses / Forbidden Remedies: None confirmed.

### batpak/src/event/header.rs
Applicable Parameters: Core-Source
Score: 88/100
Notes: Strong wire-focused header model with explicit flags and justified cast suppression. Slight reduction because semantics are still mostly documented rather than encoded as richer types.
Named Offenses / Forbidden Remedies: None confirmed.

### batpak/src/event/kind.rs
Applicable Parameters: Core-Source
Score: 90/100
Notes: Strong sealed event-kind surface with reserved-system/effect separation and good direct tests.
Named Offenses / Forbidden Remedies: None confirmed.

### batpak/src/event/mod.rs
Applicable Parameters: Core-Source
Score: 90/100
Notes: Honest event container with typed mapping and no downgrade signatures; well-backed by API tests.
Named Offenses / Forbidden Remedies: None confirmed.

### batpak/src/event/sourcing.rs
Applicable Parameters: Core-Source
Score: 82/100
Notes: Clean trait surface for replay and reaction patterns, but this file mostly defines contracts and examples rather than self-accusing proofs; doctext still shows `unwrap()` in examples.
Named Offenses / Forbidden Remedies: None confirmed.

### batpak/src/guard/denial.rs
Applicable Parameters: Core-Source
Score: 89/100
Notes: Structured denial type preserves context and supports fail-visible behavior instead of laundering failures into defaults.
Named Offenses / Forbidden Remedies: None confirmed.

### batpak/src/guard/mod.rs
Applicable Parameters: Core-Source
Score: 90/100
Notes: Strong gate composition surface with both fail-fast and evaluate-all paths, and tests prove receipts are earned rather than fabricated.
Named Offenses / Forbidden Remedies: None confirmed.

### batpak/src/guard/receipt.rs
Applicable Parameters: Core-Source
Score: 92/100
Notes: Strong sealed receipt model directly resists receipt hollowing and TOCTOU-style forgery; compile-fail tests back the invariant.
Named Offenses / Forbidden Remedies: None confirmed.

### batpak/src/id/mod.rs
Applicable Parameters: Core-Source
Score: 88/100
Notes: Clean ID abstraction with v7 generation and macro-backed semantic typing. Score reduced slightly because macro-generated semantics are harder to inspect than hand-written types.
Named Offenses / Forbidden Remedies: None confirmed.

### batpak/src/outcome/combine.rs
Applicable Parameters: Core-Source
Score: 87/100
Notes: Real algebraic combination logic with strong downstream tests; minor reduction for wildcard-arm allowance and complexity.
Named Offenses / Forbidden Remedies: None confirmed.

### batpak/src/outcome/error.rs
Applicable Parameters: Core-Source
Score: 88/100
Notes: Structured error taxonomy with explicit domain/operational/retryable classification helps resist Error Path Hollowing.
Named Offenses / Forbidden Remedies: None confirmed.

### batpak/src/outcome/mod.rs
Applicable Parameters: Core-Source
Score: 87/100
Notes: Rich algebraic result surface with strong test evidence. Score reduced for size/branch density, which increases downgrade risk even though tests currently hold it in check.
Named Offenses / Forbidden Remedies: None confirmed.

### batpak/src/outcome/wait.rs
Applicable Parameters: Core-Source
Score: 88/100
Notes: Clear semantic enums with strong serializer/property coverage and no silence patterns.
Named Offenses / Forbidden Remedies: None confirmed.

### batpak/src/pipeline/bypass.rs
Applicable Parameters: Core-Source
Score: 88/100
Notes: Bypass is explicit and justified rather than hidden, which is the right shape under this protocol. The escape hatch still deserves audit attention because it is a privileged path.
Named Offenses / Forbidden Remedies: None confirmed.

### batpak/src/pipeline/mod.rs
Applicable Parameters: Core-Source
Score: 89/100
Notes: Gate-then-commit flow is explicit and well-covered; no fake-success or local-construction issues were found.
Named Offenses / Forbidden Remedies: None confirmed.

### batpak/src/store/cursor.rs
Applicable Parameters: Core-Source
Score: 86/100
Notes: Honest guaranteed-delivery pull surface with straightforward implementation and good integration coverage.
Named Offenses / Forbidden Remedies: None confirmed.

### batpak/src/store/index.rs
Applicable Parameters: Core-Source
Score: 88/100
Notes: Real shared-state index and entity-lock ownership are clearly modeled. Slight reduction for internal complexity and reliance on indirect proofs through larger store tests.
Named Offenses / Forbidden Remedies: None confirmed.

### batpak/src/store/mod.rs
Applicable Parameters: Core-Source
Score: 80/100
Notes: This is a real orchestration core with strong end-to-end coverage, but it is very large, owns many invariants, exposes a hidden public test hook (`panic_writer_for_test`), and documents a compaction concurrency caveat instead of fully proving it away.
Named Offenses / Forbidden Remedies: Structural policy finding: visibility creep risk via public test hook.

### batpak/src/store/projection.rs
Applicable Parameters: Core-Source
Score: 76/100
Notes: Real cache backends are implemented and thoroughly tested. This module contains the `NativeCache` implementation (safe filesystem I/O via tempfile + atomic rename — the previous `LmdbCache` and its two unsafe blocks were removed in 0.3.0). There are NO unsafe blocks in projection.rs. A default `prefetch()` no-op remains and weakens the “codebase must accuse itself” posture for predictive-cache behavior.
Named Offenses / Forbidden Remedies: None confirmed; monitor for Polite Downgrade on default no-op paths.

### batpak/src/store/reader.rs
Applicable Parameters: Core-Source
Score: 85/100
Notes: Good integrity checks, FD-cache discipline, and inline unit tests. Score reduced because this module mixes production logic and test support in one file and still uses an internal `expect()` in test-only code.
Named Offenses / Forbidden Remedies: None confirmed.

### batpak/src/store/segment.rs
Applicable Parameters: Core-Source
Score: 87/100
Notes: Strong frame/segment encoding surface with explicit CRC and typestate markers; golden and edge-case tests make this a well-proved storage boundary.
Named Offenses / Forbidden Remedies: None confirmed.

### batpak/src/store/subscription.rs
Applicable Parameters: Core-Source
Score: 87/100
Notes: Thin composition layer with a clear push/pull split and direct downstream coverage in `subscription_ops.rs` and larger store tests.
Named Offenses / Forbidden Remedies: None confirmed.

### batpak/src/store/writer.rs
Applicable Parameters: Core-Source
Score: 79/100
Notes: Strong single-writer commit path and panic-recovery design, but the intentional panic path and restart logic increase structural risk, and some invariants are still comment-enforced rather than type-enforced.
Named Offenses / Forbidden Remedies: Structural policy finding: test-only panic path exists in production code.

### batpak/src/typestate/mod.rs
Applicable Parameters: Core-Source
Score: 90/100
Notes: Strong macro-based typestate generation with compile-fail coverage against invalid states.
Named Offenses / Forbidden Remedies: None confirmed.

### batpak/src/typestate/transition.rs
Applicable Parameters: Core-Source
Score: 90/100
Notes: Small, honest transition wrapper with clear compile-time semantics and good proof via typestate tests.
Named Offenses / Forbidden Remedies: None confirmed.

### batpak/tests/chaos_testing.rs
Applicable Parameters: Test-Infrastructure
Score: 82/100
Notes: STRONG. Valuable adversarial load and corruption coverage, but the file is large and uses test-only panics/unwraps/prints that increase maintenance noise.
Named Offenses / Forbidden Remedies: None confirmed.

### batpak/tests/config_propagation.rs
Applicable Parameters: Test-Infrastructure
Score: 90/100
Notes: STRONG. Good proof that configuration fields are wired through real behavior rather than orphaned in the type surface.
Named Offenses / Forbidden Remedies: None confirmed.

### batpak/tests/event_api.rs
Applicable Parameters: Test-Infrastructure
Score: 91/100
Notes: STRONG. High-value direct API tests for coordinate, event, kind, and ID semantics with strong content assertions.
Named Offenses / Forbidden Remedies: None confirmed.

### batpak/tests/fuzz_chaos_feedback.rs
Applicable Parameters: Test-Infrastructure
Score: 82/100
Notes: STRONG. Good self-measuring feedback-loop coverage, but the harness uses ignores, prints, and panic-style assertions that make the gate less deterministic than the strongest tests.
Named Offenses / Forbidden Remedies: None confirmed.

### batpak/tests/fuzz_targets.rs
Applicable Parameters: Test-Infrastructure
Score: 84/100
Notes: STRONG. Excellent serializer/fuzzer breadth and real production imports; score reduced because this file deliberately relaxes several lint rules for harness convenience.
Named Offenses / Forbidden Remedies: None confirmed.

### batpak/tests/gate_pipeline.rs
Applicable Parameters: Test-Infrastructure
Score: 90/100
Notes: STRONG. Directly proves bypass, denial, receipt, and pipeline flow semantics with concrete assertions and compile-adjacent coverage.
Named Offenses / Forbidden Remedies: None confirmed.

### batpak/tests/hash_chain.rs
Applicable Parameters: Test-Infrastructure
Score: 90/100
Notes: STRONG. Good tamper-detection and chain-integrity proof without shadow types or fake assertions.
Named Offenses / Forbidden Remedies: None confirmed.

### batpak/tests/monad_laws.rs
Applicable Parameters: Test-Infrastructure
Score: 90/100
Notes: STRONG. Focused law-based proof file with real behavioral content rather than line-coverage padding.
Named Offenses / Forbidden Remedies: None confirmed.

### batpak/tests/outcome_combinators.rs
Applicable Parameters: Test-Infrastructure
Score: 88/100
Notes: STRONG. Broad behavioral coverage of algebraic branches. Score reduced only for file size and several panic-style expected-failure assertions.
Named Offenses / Forbidden Remedies: None confirmed.

### batpak/tests/perf_gates.rs
Applicable Parameters: Test-Infrastructure
Score: 93/100
Notes: STRONG. This is the best dogfooding file in the crate: the gate system evaluates its own performance claims and produces real negative cases.
Named Offenses / Forbidden Remedies: None confirmed.

### batpak/tests/projection_cache.rs
Applicable Parameters: Test-Infrastructure
Score: 91/100
Notes: STRONG. Real backend coverage for NoCache, NativeCache, freshness, and metadata behavior closes several phantom/chimera risks.
Named Offenses / Forbidden Remedies: None confirmed.

### batpak/tests/store_advanced.rs
Applicable Parameters: Test-Infrastructure
Score: 87/100
Notes: STRONG. Excellent deep integration coverage for advanced store behaviors, but the file is very large and therefore harder to audit for gaps.
Named Offenses / Forbidden Remedies: None confirmed.

### batpak/tests/store_edge_cases.rs
Applicable Parameters: Test-Infrastructure
Score: 88/100
Notes: STRONG. Good hard-path and corruption coverage with real assertions. Score reduced slightly for local allow usage and panic-style mismatch checks.
Named Offenses / Forbidden Remedies: None confirmed.

### batpak/tests/store_integration.rs
Applicable Parameters: Test-Infrastructure
Score: 90/100
Notes: STRONG. Clear end-to-end production-path coverage for append, cold start, query, projection, CAS, and concurrency.
Named Offenses / Forbidden Remedies: None confirmed.

### batpak/tests/store_properties.rs
Applicable Parameters: Test-Infrastructure
Score: 92/100
Notes: STRONG. High-value law/property coverage for replay determinism, round-trip fidelity, idempotency, flow connectivity, and error propagation.
Named Offenses / Forbidden Remedies: None confirmed.

### batpak/tests/subscription_ops.rs
Applicable Parameters: Test-Infrastructure
Score: 62/100
Notes: WEAK-MEDIUM. Behavioral assertions are real, but this file currently breaks `cargo clippy --all-targets -- -D warnings` through a direct `panic!` and relies on crate-level allowances for `thread::spawn` and `unwrap_used`.
Named Offenses / Forbidden Remedies: Rogue Silence risk in test harness; current lint failure is active evidence.

### batpak/tests/typestate_safety.rs
Applicable Parameters: Test-Infrastructure
Score: 91/100
Notes: STRONG. Compile-fail and runtime tests together prove the typestate and receipt-forgery barriers are real.
Named Offenses / Forbidden Remedies: None confirmed.

### batpak/tests/ui/forge_receipt.rs
Applicable Parameters: Test-Infrastructure
Score: 88/100
Notes: STRONG. Small but high-value negative test that proves receipt construction is not publicly forgeable.
Named Offenses / Forbidden Remedies: None confirmed.

### batpak/tests/ui/invalid_transition.rs
Applicable Parameters: Test-Infrastructure
Score: 88/100
Notes: STRONG. Small but high-value compile-fail proof for illegal typestate transitions.
Named Offenses / Forbidden Remedies: None confirmed.

### batpak/tests/wire_format.rs
Applicable Parameters: Test-Infrastructure
Score: 91/100
Notes: STRONG. Golden-wire verification is exactly the kind of deterministic replay proof this protocol asks for.
Named Offenses / Forbidden Remedies: None confirmed.

### batpak/benches/cold_start.rs
Applicable Parameters: Examples-Benches
Score: 82/100
Notes: Honest benchmark with real store population and cold-start measurement. It is informative rather than a hard gate by itself.
Named Offenses / Forbidden Remedies: None confirmed.

### batpak/benches/compaction.rs
Applicable Parameters: Examples-Benches
Score: 80/100
Notes: Real compaction benchmark that exercises production behavior. Score reduced because it measures throughput/latency without directly proving correctness under concurrent cutover conditions.
Named Offenses / Forbidden Remedies: None confirmed.

### batpak/benches/projection_latency.rs
Applicable Parameters: Examples-Benches
Score: 86/100
Notes: Strong benchmark coverage for replay plus cache-hit/cache-miss paths across backends; `cargo bench --no-run --all-features` passed during audit.
Named Offenses / Forbidden Remedies: None confirmed.

### batpak/benches/subscription_fanout.rs
Applicable Parameters: Examples-Benches
Score: 81/100
Notes: Useful fan-out benchmark against the real store/writer path. It remains a measurement artifact rather than a policy gate.
Named Offenses / Forbidden Remedies: None confirmed.

### batpak/benches/write_throughput.rs
Applicable Parameters: Examples-Benches
Score: 84/100
Notes: Broad throughput benchmark with concurrency coverage and real append paths; stronger than a toy microbenchmark.
Named Offenses / Forbidden Remedies: None confirmed.

### batpak/examples/chat_room.rs
Applicable Parameters: Examples-Benches
Score: 70/100
Notes: Clear runnable example with real API usage, but examples are explanatory surfaces, not proving artifacts, and this one relies heavily on printed narration.
Named Offenses / Forbidden Remedies: None confirmed.

### batpak/examples/dungeon_typestate.rs
Applicable Parameters: Examples-Benches
Score: 72/100
Notes: Good pedagogical typestate example using real APIs. Score reduced because behavior is narrated through prints rather than checked as an invariant-bearing test.
Named Offenses / Forbidden Remedies: None confirmed.

### batpak/examples/event_sourced_counter.rs
Applicable Parameters: Examples-Benches
Score: 74/100
Notes: Good event-sourcing walkthrough with real projection/replay usage. Limited self-accusation because it is demonstrative, not assertive.
Named Offenses / Forbidden Remedies: None confirmed.

### batpak/examples/policy_gates.rs
Applicable Parameters: Examples-Benches
Score: 76/100
Notes: Strongest example of the set because it demonstrates real approval/denial paths, but it is still observational rather than test-enforced.
Named Offenses / Forbidden Remedies: None confirmed.

### Crate Rollup: batpak
Score: 83/100
Notes: The crate is materially strong on self-accusing tests, compile-fail proofs, golden wire checks, and build-time invariant enforcement. The main unresolved findings are operational: `batpak/.cargo/config.toml` hardcodes a local linker path, `batpak/deny.toml` does not parse under the local `cargo deny check`, `batpak/src/store/mod.rs` and `batpak/src/store/writer.rs` carry test-only/public-surface risk, and `batpak/tests/subscription_ops.rs` currently fails the stronger clippy gate that the repo claims to want.

## Repo / System / DevOps

### .editorconfig
Applicable Parameters: Build-Config
Score: 72/100
Notes: Useful formatting baseline, but it is a light hygiene control rather than a meaningful integrity detector.
Named Offenses / Forbidden Remedies: None confirmed.

### README.md
Applicable Parameters: Spec-Docs
Score: 74/100
Notes: Good top-level navigation and project framing, but it is index-like rather than traceability-rich.
Named Offenses / Forbidden Remedies: None confirmed.

### CONTRIBUTING.md
Applicable Parameters: Spec-Docs
Score: 70/100
Notes: Clear contributor workflow, but it overstates lint confidence by recommending clippy without the stronger all-targets form that exposes the current `subscription_ops.rs` failure.
Named Offenses / Forbidden Remedies: Coverage Mirage risk on contributor guidance.

### SPEC.md
Applicable Parameters: Spec-Docs
Score: 62/100
Notes: This is the strongest design document conceptually, but it has drifted: it still references `tests/self_benchmark.rs` and other historical paths, so bidirectional traceability is no longer trustworthy as written.
Named Offenses / Forbidden Remedies: Bidirectional traceability failure.

### SPEC_REGISTRY.md
Applicable Parameters: Spec-Docs
Score: 58/100
Notes: Similar to `SPEC.md`, this is rich in intended architecture but now contains stale file references and therefore undermines architecture freeze fidelity.
Named Offenses / Forbidden Remedies: Bidirectional traceability failure.

### scripts/verify-all.sh
Applicable Parameters: DevOps-Scripts
Score: 66/100
Notes: Helpful wrapper script for fmt/clippy/test/doc, but it does not run clippy on all targets, does not include the deny gate, and therefore does not fully “accuse the codebase” under the stronger standard.
Named Offenses / Forbidden Remedies: Coverage Mirage risk on local verification script.

### Repo / System / DevOps Rollup
Score: 67/100
Notes: The repo-level surfaces explain the system well, but they lag the implemented crate. The dominant issue is traceability drift in `SPEC.md` and `SPEC_REGISTRY.md`, followed by underpowered lint/dependency gates in `CONTRIBUTING.md` and `scripts/verify-all.sh`.

## Aggregate Score
82/100

## 2026-04-11 Red Team Addendum — Full-Send Hardening Pass

This addendum is append-only. Earlier findings remain part of the audit record even
when they no longer reproduce in the current tree. Findings closed in this pass are
called out as `Superseded in current tree`; they are preserved here because the audit
history matters as much as the current state.

### Preserved Prior Findings

- Prior finding: H1 broken hash chain for multi-item same-entity batches.
  Severity: HIGH.
  Surface: `src/store/writer.rs` atomic batch path.
  Why easy to miss: the drift lived across two folds over the same batch state.
  Relation to prior findings: original marquee v0.3.0 defect.
  Current status: `Superseded in current tree`.
  Closure in this pass: the per-item material needed downstream stays fused in
  `BatchItemComputed`, and the same-key batch scratch maps were further collapsed
  into one stateful path instead of parallel runtime facts.

- Prior finding: H2 committed batch lost on cold start without SIDX.
  Severity: HIGH.
  Surface: `src/store/reader.rs` slow-path recovery.
  Why easy to miss: the false premise was plausible; durable commit and footer
  publication were treated as the same thing.
  Relation to prior findings: original marquee v0.3.0 defect.
  Current status: `Superseded in current tree`.
  Closure in this pass: slow-path rebuild now streams scan results directly into
  replay insertion while preserving COMMIT-marker durability semantics and
  cross-segment batch recovery state.

- Prior finding: M1 wall clock regression could reorder batch-visible state.
  Severity: MEDIUM.
  Surface: batch write path.
  Why easy to miss: wall-clock and index-ordering invariants were maintained in
  separate places.
  Relation to prior findings: same drift-prone batch family as H1/H2.
  Current status: `Superseded in current tree`.
  Closure in this pass: per-item `wall_ms` remains precomputed once and reused
  verbatim through write and stage phases.

- Prior finding: M2 `NativeCache::sync()` looked meaningful while being a no-op.
  Severity: MEDIUM.
  Surface: projection cache semantics and docs.
  Why easy to miss: the API shape looked durability-bearing even though the backend
  never promised that property.
  Relation to prior findings: delivery/freshness semantics that looked safer than they were.
  Current status: bounded, documented, and now paired with stricter naming elsewhere.

### Dangerous Defaults And Misleading Examples

#### Finding A1 — Ambiguous delivery names looked stronger than they were
- Severity: MEDIUM
- Surface: `Store::subscribe()` / `Store::cursor()` public API
- Why easy to miss: the old names hid the safety distinction behind docs instead of the type surface
- Exploit / failure story: wrappers pick the push API assuming replay semantics, drop notifications under pressure, and silently miss events
- Preconditions: caller embeds batpak in a service and treats bounded broadcast as guaranteed delivery
- Impact: silent data loss at the wrapper boundary
- Evidence in current tree: renamed to `subscribe_lossy()` and `cursor_guaranteed()` in `src/store/mod.rs`, docs, examples, tests, benches, and spec
- Relation to prior findings: same honesty gap as the old freshness naming
- Closure in this pass: closed

#### Finding A2 — Freshness enum overstated safety
- Severity: MEDIUM
- Surface: projection cache freshness contract
- Why easy to miss: `BestEffort` sounds benevolent, not stale
- Exploit / failure story: product code picks it in correctness-sensitive paths and accidentally serves stale state
- Preconditions: caller uses cached projections without understanding bounded staleness
- Impact: stale reads dressed up as a neutral default
- Evidence in current tree: `Freshness::MaybeStale { max_stale_ms }` in code, docs, tests, and spec
- Relation to prior findings: sibling of A1
- Closure in this pass: closed

### Lifecycle And Durability Traps

#### Finding L1 — Drop looked too much like clean shutdown
- Severity: HIGH
- Surface: store lifecycle
- Why easy to miss: the old API relied on users noticing shutdown caveats in prose
- Exploit / failure story: service wrappers rely on destructor timing, lose the last burst of writes, and discover it only under restart or deploy pressure
- Preconditions: caller drops `Store` without explicit `close(self)`
- Impact: bounded but real write loss window and misleading operational behavior
- Evidence in current tree: explicit `close(self)` remains the clean contract, `Drop` now warns loudly, specs/docs/examples emphasize the distinction, and close returns a terminal `Closed` token
- Relation to prior findings: same “looks safer than it is” family as delivery naming
- Closure in this pass: bounded and made operationally loud

### Delivery / Freshness Semantics That Looked Safer Than They Were

#### Finding D1 — `watch_projection` replayed from genesis on every notification
- Severity: HIGH
- Surface: `ProjectionWatcher`
- Why easy to miss: the code was functionally correct on small entities and the subscription path hid the amplification shape
- Exploit / failure story: a hot entity with a long history plus a burst of notifications turns one append stream into repeated whole-history scans
- Preconditions: large entity histories, frequent updates, watcher consumers
- Impact: CPU amplification and latency collapse
- Evidence in current tree: watcher-local cached projection state plus index watermark tailing in `src/store/mod.rs`, and internal `StoreIndex::stream_since()` in `src/store/index.rs`
- Relation to prior findings: canonical banana-split target from the red-team pass
- Closure in this pass: closed

#### Finding D2 — Lossy subscriptions could have hidden watcher convergence gaps
- Severity: MEDIUM
- Surface: watcher update path
- Why easy to miss: “I got a notification” can be mistaken for “I saw every event”
- Exploit / failure story: bounded broadcast drops intermediate notifications and watcher state silently lags if it only applies one event per recv
- Preconditions: backpressure or bursty append traffic
- Impact: stale or incorrect watcher state
- Evidence in current tree: watcher refresh is watermark-based rather than single-notification-based, and `watch_projection_catches_up_after_lossy_notifications` was added to `tests/unified_red.rs`
- Relation to prior findings: direct follow-on from D1
- Closure in this pass: closed

### Hostile Filesystem / Path-Trust / Temp-File Footguns

#### Finding F1 — Predictable checkpoint temp path allowed symlink clobber setup
- Severity: HIGH
- Surface: checkpoint persistence
- Why easy to miss: the vulnerable pattern looked like routine atomic-write plumbing
- Exploit / failure story: attacker pre-places a symlink at the fixed temp leaf, waits for checkpoint write, and redirects or clobbers an unintended file
- Preconditions: attacker can create entries in `data_dir`
- Impact: file clobber / checkpoint corruption
- Evidence in current tree: `src/store/checkpoint.rs` now uses `NamedTempFile::new_in(data_dir)` and rejects symlink leaf targets; coverage added in `tests/store_edge_cases.rs`
- Relation to prior findings: foundational hostile-filesystem issue from the audit lanes
- Closure in this pass: closed

#### Finding F2 — Native-cache persistence had the same predictable temp-file shape
- Severity: MEDIUM
- Surface: `NativeCache::put`
- Why easy to miss: cache code often gets excused as “non-durable anyway”
- Exploit / failure story: attacker redirects cache writes or uses a malicious cache leaf to create integrity surprises on a shared mount
- Preconditions: attacker controls or can pre-seed cache root paths
- Impact: cache corruption, path traversal surprise, operational confusion
- Evidence in current tree: `src/store/projection.rs` uses same-directory `NamedTempFile`, rejects symlink leafs, and has an explicit symlink-rejection test
- Relation to prior findings: sibling of F1
- Closure in this pass: closed

### Amplification / Resource Abuse / Operator-Mistake Traps

#### Finding R1 — Single append size was effectively bounded only by `u32::MAX`
- Severity: HIGH
- Surface: append boundary
- Why easy to miss: batch append had a real operational limit, making single-append asymmetry less obvious
- Exploit / failure story: caller sends a giant single payload, forcing serialization and memory pressure far beyond the intended operational envelope
- Preconditions: untrusted caller payloads or missing wrapper-side limits
- Impact: memory pressure, latency spikes, crash risk
- Evidence in current tree: `StoreConfig::single_append_max_bytes`, validation in `src/store/mod.rs`, and regression coverage in `tests/store_edge_cases.rs`
- Relation to prior findings: classic “basic thing easy to overlook” item from the follow-up audit pass
- Closure in this pass: closed

#### Finding R2 — Unbounded coordinate component size fed interner abuse
- Severity: MEDIUM
- Surface: `Coordinate::new`
- Why easy to miss: emptiness checks existed, so the constructor looked “validated enough”
- Exploit / failure story: attacker generates huge entity/scope components, multiplying memory pressure and index churn
- Preconditions: caller controls entity/scope strings
- Impact: memory amplification and degraded index locality
- Evidence in current tree: fixed component-length checks in `src/coordinate/mod.rs` and regression coverage in `tests/store_edge_cases.rs`
- Relation to prior findings: same resource-abuse family as R1
- Closure in this pass: closed

#### Finding R3 — `react_loop` paid an extra disk read per committed event
- Severity: MEDIUM
- Surface: reactor pipeline
- Why easy to miss: the old logic was functionally correct and the redundant read lived behind a lightweight notification abstraction
- Exploit / failure story: high-volume reactor workloads turn every reaction into extra disk I/O, amplifying the cost of already-committed events
- Preconditions: callers use `react_loop` on active workloads
- Impact: unnecessary I/O and a larger blast radius under load
- Evidence in current tree: private `CommittedEventEnvelope` in `src/store/writer.rs` and `react_loop` consuming that envelope directly in `src/store/mod.rs`
- Relation to prior findings: banana-split fusion by carrying the richer accumulator once
- Closure in this pass: closed

### Tooling / Supply Chain / Verification Blind Spots

#### Finding T1 — advisories were visible but not release-blocking
- Severity: HIGH
- Surface: `cargo xtask deny` / release flow / CI parity
- Why easy to miss: a warn-only gate still prints scary output, which looks better than it is
- Exploit / failure story: maintainers believe dependency advisories are enforced while CI keeps shipping through them
- Preconditions: advisory-db hit or deny parser issue
- Impact: vulnerable dependency drift can reach release candidates
- Evidence in current tree: `cargo xtask deny` now runs hard `cargo deny check` plus hard `cargo audit --deny warnings`; `cargo-audit` pin added to xtask setup, CI, devcontainer, and integrity parity checks
- Relation to prior findings: direct closure of the earlier supply-chain blind spot
- Closure in this pass: closed, with one explicit narrow allowlist for `RUSTSEC-2026-0097` because the remaining vulnerable `rand 0.9.2` edge is upstream in `proptest`'s dev-only dependency graph after batpak's own runtime/test-hook code was moved off `rand`

#### Finding T2 — structural drift toward old misleading names could reappear by copy-paste
- Severity: MEDIUM
- Surface: build/integrity guardrails
- Why easy to miss: regressions usually re-enter through docs and convenience refactors, not the original bug site
- Exploit / failure story: a future edit reintroduces `subscribe()`, `cursor()`, `test-support`, or fixed temp-file patterns and the repo quietly starts lying again
- Preconditions: ordinary maintenance drift
- Impact: semantics regress faster than reviewers notice
- Evidence in current tree: new build-time and integrity-time checks for stale surface names and fixed temp-file patterns in `build.rs` and `tools/integrity/src/main.rs`
- Relation to prior findings: “idiot-proof us too” hardening from the second follow-up pass
- Closure in this pass: closed

### Banana-Split And Bulkhead Refactors That Removed Drift-Prone Parallel State

- `ProjectionWatcher` is now the clearest banana-split win in the tree: one full fold, then watermark-based delta application instead of replay-from-genesis on every notification.
- Cold-start rebuild now fuses scan and replay insertion: `scan_segment_index_into()` emits entries directly into the replay cursor instead of allocating per-segment intermediate vectors first.
- Batch append continues to preserve the crucial durability boundary of `precompute -> write -> fsync -> stage/publish`, but the per-item facts that must not drift (`prev_hash`, `event_hash`, `clock`, `wall_ms`, causation, event id) are computed once and reused verbatim downstream.
- Reactor handling now carries the already-committed event envelope once instead of shipping a lightweight notification and forcing a second fold back through disk.

### Status Updates On Prior Findings

- H1 broken same-entity batch hash chain: `Superseded in current tree`
- H2 committed batch dropped on cold start without SIDX: `Superseded in current tree`
- M1 wall clock regression in batch path: `Superseded in current tree`
- M2 misleading native-cache durability expectation: bounded, documented, and no longer paired with euphemistic freshness naming

### Prioritized Hardening Threads Closed In This Pass

- Hostile temp-file and symlink leaf handling for checkpoint and native cache
- Real single-event payload cap
- Fixed coordinate component length caps
- Explicit lossy vs guaranteed delivery naming
- Explicit maybe-stale freshness naming
- Open-state lifecycle typestate plus explicit `Closed` terminal result
- Watermark-based watcher catch-up
- Scan-to-sink cold-start rebuild fusion
- Private richer reactor envelope to remove redundant event rereads
- Hard advisory gating in xtask / CI / release flow
