# Contributing to `batpak`

## Canonical Environment

The checked-in devcontainer is the canonical environment. Native Windows and Linux are supported too, but they should use the same root-first commands:

```bash
cargo xtask doctor
cargo xtask ci
```

If your local toolchain is missing standard cargo helpers, run:

```bash
cargo xtask setup --install-tools
```

## Daily Commands

```bash
cargo xtask doctor
cargo xtask traceability
cargo xtask structural
cargo xtask pre-commit
cargo xtask ci
```

`just` recipes are wrappers around the same commands.

## Contributor Workflow

1. Make the change.
2. Update docs, examples, traceability, and ADRs if the public surface or behavior changed.
3. Run `cargo xtask pre-commit`.
4. Run `cargo xtask preflight` before pushing. This runs CI inside the canonical devcontainer — bit-equivalent to the GH `Integrity (ubuntu-devcontainer)` job — so it eliminates "passes locally, fails CI" surprises that `cargo xtask ci` on a native host cannot catch (different toolchain, missing system deps, wrong env vars). Use `cargo xtask ci` as a faster inner-loop check during iterative development, but always finish with `preflight` before the push that matters.

## Public Surface Rules

- No async runtime dependency in production.
- No `unwrap()`, `todo!()`, `dbg!()`, or `panic!()` in production code.
- Keep host-specific linker and machine overrides out of the repo.
- New public APIs should have:
  - a doc comment or guide entry
  - an example or test by name
  - traceability updates when behavior or invariants change

## Docs And Release Hygiene

- User-facing docs live in `guide/`
- Deep reference docs live in `docs/reference/`
- ADRs live in `docs/adr/`
- Specs and audit records live in `docs/spec/` and `docs/audits/`

Before release-oriented changes, run:

```bash
cargo xtask docs
cargo xtask release --dry-run
```
