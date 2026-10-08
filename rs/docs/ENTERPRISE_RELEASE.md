# Enterprise Release Contract

RamShield is production-qualified only when source, artifact, deployment, and runtime evidence agree.

- Git tag equals root Cargo version.
- fmt/check/clippy/tests/audit are green on the pinned toolchain.
- Release archives have SHA-256 checksums and Sigstore signatures.
- SPDX SBOM and build provenance are published.
- Installer requires an explicit release version and verifies the artifact before installation.
- Upgrade and rollback qualification fail closed on enforcement-state loss.
- Kubernetes images are pinned to a release tag; `latest` is not a production reference.
- Stock privileged control surfaces remain loopback-only.

RamShield protects the host/network edge. It does not provide upstream link capacity or replace upstream DDoS scrubbing.
