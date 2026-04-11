# Agent Guide

## Repo Map

- `src/`: runtime crate
  - `src/store/`: see `mod.rs` for full submodule list. Key internals:
    - `checkpoint.rs` — index checkpoint (fast cold-start persistence)
    - `sidx.rs` — SIDX segment footer for cold-start rebuild
    - `columnar.rs` — SoA/AoSoA/SoAoS secondary query index
    - `interner.rs` — string interning for compact index keys
    - `projection_flow.rs` — projection replay + incremental apply + schema versioning
    - `fault.rs` — fault injection framework for chaos testing (test-support feature)
    - `writer.rs` — background writer thread, single/batch append, two-phase commit
    - `contracts.rs` — `BatchAppendItem`, `CausationRef`, `AppendOptions`
- `tests/`: integration, property, compile-fail, and perf-gate tests (30 files)
- `examples/`: runnable usage patterns
- `benches/`: Criterion surfaces
- `tools/integrity/`: traceability and structural detectors
- `tools/xtask/`: canonical developer command surface
- `guide/`: newcomer-facing docs
- `docs/reference/`: deeper narrative and tuning docs
- `docs/adr/`: decision records
- `traceability/`: requirements, invariants, flows, artifacts

## Canonical Commands

- `cargo xtask doctor`
- `cargo xtask preflight`     — full CI inside the canonical devcontainer (gold standard before pushing); bit-equivalent to the GH `Integrity (ubuntu-devcontainer)` job, so "passes locally fails CI" surprises are eliminated. Prefer this over bare `cargo xtask ci` for any push that touches store internals, xtask itself, or CI config.
- `cargo xtask ci`
- `cargo xtask perf-gates`    — hardware-dependent perf gate tests; run only on stable hardware. Excluded from `cargo xtask ci` because they use `Instant::now()` and flake on shared CI runners.
- `cargo xtask bench --surface neutral|native`
- `cargo xtask docs`
- `cargo xtask release --dry-run`

## Change Map

- Public API change:
  - update docs or guide
  - update examples if onboarding changed
  - update traceability and ADRs if invariants/flows changed
- Store internals change:
  - run `cargo xtask ci`
  - run the relevant perf surface
  - inspect `tests/perf_gates.rs` and `docs/reference/TUNING.md`
- Benchmark harness change:
  - update `scripts/bench-report`
  - refresh baselines intentionally
  - keep backend-neutral vs backend-specific surfaces honest
- Docs-only change:
  - keep root README, guide, and reference docs consistent

## Guardrails

- Do not introduce async runtime dependencies in production.
- Keep root-first commands and paths accurate.
- If you add a public item or named flow, update `traceability/`.
- Prefer `cargo xtask` over inventing new one-off local commands.
- **Structural parity checks** — `cargo xtask structural` (called automatically by `cargo xtask ci`) runs two detectors you must not break:
  - `check_ci_parity` — fails if `.github/workflows/ci.yml` drifts from `tools/xtask/src/main.rs` or `.devcontainer/Dockerfile`. Specifically: every `cargo xtask <subcommand>` referenced in the workflow must exist as a subcommand in xtask; every `taiki-e/install-action` tool must be present in xtask's setup step; tool version pins must agree across all three files. **Rule:** if you modify `tools/xtask/src/main.rs`, `.github/workflows/ci.yml`, or `.devcontainer/Dockerfile`, run `cargo xtask structural` before push.
  - `check_store_pub_fn_coverage` — uses `syn` to parse `src/store/mod.rs`, extracts every `pub fn` on `impl Store`, and asserts that each one has at least one method-call reference somewhere in `tests/` or `src/`. Catches orphan public methods that ship untested and invisible to mutation testing. **Rule:** if you add a `pub fn` to `Store`, ensure it has a call site in tests or the check will fail.

## Mutation Testing Gate

The `mutants` job in `ci.yml` runs on every `push` and `pull_request` — it is **not** report-only. `cargo-mutants 27.0` exits non-zero on any missed mutation. Additionally, `tools/xtask/src/main.rs::assert_mutation_score` enforces a >= 20% catch rate as a percentage-threshold backup. Removing tests without replacement will fail the PR.

**Rule:** if you delete a test, expect the mutation score to drop; either replace it with an equivalent test or write a stronger one that subsumes its coverage.

## Test-Authoring Caveats

**`expect_err` is off-limits for `Store` and `Receipt` results.** The audit found five agent-authored sites that reached for `Result::expect_err`, which requires `T: Debug` on the `Ok` variant. Neither `Store` nor `Receipt<&str>` implements `Debug`. Use the explicit-panic pattern instead:

```rust
let err = match result {
    Ok(_) => panic!("PROPERTY: expected an error here but got Ok"),
    Err(e) => e,
};
assert!(matches!(err, StoreError::SpecificVariant { .. }), "wrong variant: {:?}", err);
```

Test files that use `panic!()` intentionally (as the loop-escape in property tests) need `#![allow(clippy::panic)]` at the module level. The project's `Cargo.toml` denies `panic` globally for `src/`, but test files use it on purpose and must opt out explicitly.

**Extract local visitor structs to module level for testability.** Visitor structs defined inside a function body (e.g., `U128Visitor`, `OptU128Visitor`, `VecU128Visitor` in `src/wire.rs`) are unreachable from `tests/` and invisible to mutation testing — mutations inside them go undetected. The fix is to move them to `pub(super) struct` at module level. Apply this pattern whenever you define a `serde::Visitor` or similar helper inside a function: the slight verbosity is worth the coverage gain.
