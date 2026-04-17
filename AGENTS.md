# Agent Guide

## Repo Map

- `src/`: runtime crate
  - `src/store/`: see `mod.rs` for full submodule list. Key subdirectories:
    - `write/` — `writer.rs` (background writer, single/batch commit), `fanout.rs` (notifications), `staging.rs`, `control.rs`
    - `segment/` — `mod.rs` (frame format, compaction), `scan.rs` (segment reading), `sidx.rs` (SIDX footer)
    - `index/` — `mod.rs` (in-memory query engine), `columnar.rs` (SoA/AoSoA overlays), `interner.rs` (string interning)
    - `cold_start/` — `mod.rs` (open/restore orchestration), `checkpoint.rs`, `mmap.rs`, `rebuild.rs`
    - `projection/` — `mod.rs` (cache traits), `flow.rs` (replay + incremental apply), `watch.rs`
    - `ancestry/` — `mod.rs`, `by_hash.rs`, `by_clock.rs`
    - `delivery/` — `subscription.rs` (lossy push), `cursor.rs` (guaranteed pull)
    - Flat files: `append.rs` (`BatchAppendItem`, `CausationRef`, `AppendOptions`), `lifecycle.rs`, `hidden_ranges.rs`, `config.rs`, `error.rs`, `stats.rs`
    - `fault.rs` — fault injection (dangerous-test-hooks feature)
- `tests/`: integration, property, compile-fail, and perf-gate tests (30 files)
- `examples/`: runnable usage patterns
- `benches/`: Criterion surfaces
- `tools/integrity/`: traceability and structural detectors
- `tools/xtask/`: canonical developer command surface
- `README.md`: primary repo entrypoint
- `GUIDE.md`: human-first workflows and usage
- `REFERENCE.md`: technical reference and invariants
- `docs/adr/`: decision records
- `traceability/`: requirements, invariants, flows, artifacts

## Canonical Commands

- `cargo xtask doctor`
- `cargo xtask install-hooks`
- `cargo xtask preflight`     — full proof chain inside the canonical devcontainer, entered once per run (gold standard before pushing); the closest local match to the GH `Integrity (ubuntu-devcontainer)` lane because it runs CI, coverage, and docs from one in-container session. Prefer this over bare `cargo xtask ci` for any push that touches store internals, xtask itself, or CI config.
- `cargo xtask ci`
- `cargo xtask perf-gates`    — hardware-dependent catastrophic-regression guards, not precision perf gates. Run only on stable hardware; no current environment is both canonical and timing-stable, so these thresholds stay intentionally generous and are excluded from `cargo xtask ci`.
- `cargo xtask bench --surface neutral|native [--save|--compare|--compile]`
- `cargo xtask cover [--ci|--json|--threshold N]`
- `cargo xtask docs`
- `cargo xtask release --dry-run`

## Change Map

- Public API change:
  - update README, GUIDE, or REFERENCE as appropriate
  - update examples if onboarding changed
  - update traceability and ADRs if invariants/flows changed
- Store internals change:
  - run `cargo xtask ci`
  - run the relevant perf surface
  - inspect `tests/perf_gates.rs` and `REFERENCE.md`
- Benchmark harness change:
  - update `cargo xtask bench` surfaces in `tools/xtask/src/bench.rs`
  - refresh baselines intentionally
  - keep backend-neutral vs backend-specific surfaces honest
- Coverage harness change:
  - update `tools/xtask/src/coverage.rs`
  - keep JSON mode stdout-clean
  - keep retained artifacts under `target/xtask-cover/last-run/`
- Docs-only change:
  - keep root README, GUIDE, and REFERENCE consistent

## Guardrails

- Do not introduce async runtime dependencies in production.
- Keep root-first commands and paths accurate.
- If you add a public item or named flow, update `traceability/`.
- Prefer `cargo xtask` over inventing new one-off local commands.
- `.githooks/` is the tracked repo hook surface. `cargo xtask setup --install-tools` will install it when no custom `core.hooksPath` is active; otherwise use `cargo xtask install-hooks` after clearing or changing the custom hook path.
- **Structural parity checks** — `cargo xtask structural` (called automatically by `cargo xtask ci`) runs two detectors you must not break:
  - `check_ci_parity` — fails if `.github/workflows/ci.yml` drifts from the xtask source tree or `.devcontainer/Dockerfile`. Specifically: every `cargo xtask <subcommand>` referenced in the workflow must exist as a subcommand in xtask; every `taiki-e/install-action` tool must be present in xtask's setup step; tool version pins must agree across all three files. **Rule:** if you modify `tools/xtask/src/main.rs`, `tools/xtask/src/commands.rs`, `.github/workflows/ci.yml`, or `.devcontainer/Dockerfile`, run `cargo xtask structural` before push.
  - `check_store_pub_fn_coverage` — uses `syn` to parse `src/store/mod.rs`, extracts every `pub fn` on `impl Store`, and asserts that each one has at least one method-call reference somewhere in `tests/` or `src/`. Catches orphan public methods that ship untested and invisible to mutation testing. **Rule:** if you add a `pub fn` to `Store`, ensure it has a call site in tests or the check will fail.

## Mutation Testing Gate

The `mutants` job in `ci.yml` runs on every `push` and `pull_request` — it is **not** report-only. `cargo-mutants 27.0` exits non-zero on any missed mutation. Additionally, `tools/xtask/src/commands.rs::assert_mutation_score` enforces a >= 20% catch rate as a percentage-threshold backup. Removing tests without replacement will fail the PR.

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
