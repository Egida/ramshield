# RamShield security model

## Assets

The primary asset is enforcement truth: which IPs/CIDRs are blocked and for how long. Secondary assets are IPC credentials, dashboard credentials, WAL contents and kernel projection state.

## Trust boundaries

```text
external traffic
   ↓
proxy / telemetry producer        untrusted input
   ↓
IPC protocol                      authenticated boundary
   ↓
detection                        bounded state
   ↓
enforcement + WAL                authoritative state
   ↓
XDP / SHM                         kernel/local projections
```

The telemetry path must be treated as hostile data. IPC framing, authentication, replay protection, timeouts and size bounds are part of the boundary.

## Failure policy

RamShield prefers explicit degradation over false claims of protection. Examples:

- WAL failure with volatile fallback disabled → startup failure.
- required XDP attach failure → startup failure.
- explicit in-band fallback → degraded health, not healthy XDP.
- XDP mutation failure while active → projection stale immediately.
- SHM projection saturation → publication failure; userspace remains authoritative.

## Secrets

Treat IPC HMAC keys and dashboard credential material as secrets. Do not put them in the repository or command history. Public control-plane binds require authentication and a TLS/mTLS boundary.

## Linux privilege

The daemon needs elevated capabilities only for the dataplane features that require them. Systemd and Kubernetes profiles should grant the minimum capabilities needed by the selected mode. XDP qualification must include the actual kernel, driver and capability set.

## Supply chain

Cargo uses a committed lockfile and release builds use `--locked`. CI runs dependency auditing and cargo-deny. Release archives receive checksums, signatures, SBOM and provenance. GitHub recommends immutable full-SHA action references for stronger workflow supply-chain protection; action pinning is therefore a release-policy gate. [GitHub Docs](https://docs.github.com/en/actions/reference/security/secure-use?presscats=25&utm_source=chatgpt.com) Cargo's `--locked` behavior is also the correct deterministic release posture because it refuses dependency-resolution changes. [Rust Documentation](https://doc.rust-lang.org/cargo/commands/cargo-build.html?utm_source=chatgpt.com)

## Security reporting

Follow `SECURITY.md` for private vulnerability reporting. Do not publish an exploitable security issue before the maintainer has had a reasonable opportunity to assess and remediate it.