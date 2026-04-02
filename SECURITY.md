# Security Policy

Please report suspected vulnerabilities privately through the repository security advisory flow or by contacting the maintainers directly before opening a public issue.

Security-sensitive changes should preserve:

- sync-only production runtime
- explicit trust boundaries
- append-only event durability
- hash-chain integrity when `blake3` is enabled
- traceability and auditability of behavior changes
