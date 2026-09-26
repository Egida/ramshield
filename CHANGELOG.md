# Changelog

Notable user-facing changes are recorded here.

The format follows [Keep a Changelog](https://keepachangelog.com/) and releases use Semantic Versioning.

## [Unreleased]

### Added

- Authenticated IPC with HMAC-SHA256.
- Key identity and role-based IPC authorization.
- Replay protection for authenticated IPC frames.
- Safer public IPC binding through `behind_tls_proxy`.
- WAL replay idempotency qualification.
- XDP reconciliation qualification.
- Explicit enforcement-queue backpressure errors.
- `--no-xdp` daemon flag for local and CI runs.
- Production-like smoke test script.
- Argon2-protected dashboard sessions and Prometheus `/metrics`.
- Configurable dashboard block history size.
- Forecasting and detection improvements including SPOT-lite, EWMA/CUSUM work, and pulse detection.

### Changed

- Subnet blocking now uses distinct source IPs as one gate, reducing whole-subnet reactions to a single burst.
- Subnet-burst blocks use a shorter TTL than ordinary IP blocks.
- Protocol requests reject unknown fields instead of silently accepting them.
- Dead legacy modules/codecs were removed.

### Fixed

- Oversize IPC connections now receive a typed error before close.
- Lock-poisoning handling was hardened across storage paths.

## [0.2.0] - 2026-07-31

### Added

- Initial project release notes and contributor guidance.

### Changed

- Build and verification guidance was tightened.

### Fixed

- Removed dead imports and unused constants that blocked verification.

[Unreleased]: https://github.com/grep999/ramshield/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/grep999/ramshield/releases/tag/v0.2.0