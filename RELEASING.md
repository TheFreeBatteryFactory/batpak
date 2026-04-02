# Releasing `batpak`

## Before Tagging

```bash
cargo xtask ci
cargo xtask docs
cargo xtask release --dry-run
```

## Release Checklist

1. Update `CHANGELOG.md`
2. Verify docs and examples still match the public API
3. Confirm guide and rustdoc build cleanly
4. Create a version/tag PR if needed
5. Use the release workflow or tag-based publish flow
