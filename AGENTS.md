# Agent Guide

## Repo Map

- `src/`: runtime crate
- `tests/`: integration, property, compile-fail, and perf-gate tests
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
- `cargo xtask ci`
- `cargo xtask bench --surface neutral|redb|lmdb`
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
