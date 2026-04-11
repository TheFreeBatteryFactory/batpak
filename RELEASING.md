# Releasing `batpak`

## Before Tagging

```bash
cargo xtask preflight
cargo xtask ci
cargo xtask docs
cargo xtask release --dry-run
```

`cargo xtask preflight` runs CI inside the canonical devcontainer and must be the first check for a release: a tag is permanent, so it is more important to catch environment-specific failures before tagging than during ordinary development. Host-only `cargo xtask ci` is not sufficient here.

## Release Checklist

1. Update `CHANGELOG.md`
2. Verify docs and examples still match the public API
3. Confirm guide and rustdoc build cleanly
4. Create a version/tag PR if needed
5. Use the release workflow or tag-based publish flow
