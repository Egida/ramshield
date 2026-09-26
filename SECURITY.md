# Security

RamShield is a security-sensitive system component. Treat the network-facing interfaces and the XDP process as privileged boundaries.

## Reporting a vulnerability

Please do not open a public issue for a security vulnerability.

Report vulnerabilities to **autodafeyolo@gmail.com** with enough detail to reproduce the issue. The project aims to acknowledge reports within 48 hours and provide an update on investigation and remediation.

## Current security model

### IPC

IPC uses HMAC-SHA256 authenticated frames when keys are configured. The protocol also supports key roles and replay protection.

Authentication is not encryption. A network observer can still see IPC traffic unless the connection is kept on loopback or protected by a TLS/mTLS proxy.

For exposed IPC:

- bind only to a trusted network,
- configure `auth_keys`,
- use role-based keys,
- set `behind_tls_proxy = true` only when a real TLS/mTLS boundary is in front.

### Dashboard

The dashboard supports an Argon2 password hash and session-cookie authentication. Config changes have CSRF checks.

RamShield does not provide a built-in TLS server. Do not expose the dashboard directly to an untrusted network without an appropriate TLS boundary.

The safest default is the loopback bind:

```toml
[dashboard]
http_addr = "127.0.0.1:9999"
```

### XDP

XDP requires elevated kernel capabilities. The current documented runtime set is:

```text
CAP_NET_ADMIN
CAP_BPF
CAP_PERFMON
```

Apply them to the actual release binary when needed:

```bash
sudo setcap 'cap_net_admin,cap_perfmon,cap_bpf+eip' target/release/ramshield
```

A rebuild replaces the binary and therefore requires the capabilities to be applied again.

### Persistent state

When WAL is enabled, block state is replayed during startup and restored block expirations are re-armed. WAL failures are surfaced in logs; operators should not assume persistence is working merely because the process is running.

## Security boundaries and limitations

RamShield does not provide:

- transport encryption by itself;
- protection against attacks that saturate the upstream link;
- a replicated multi-node control plane;
- a guarantee that every detected block reaches the kernel;
- immunity from false positives, especially around shared/CGNAT addresses.

## Operational guidance

Prefer:

1. loopback IPC/dashboard for local deployments;
2. a dedicated service user;
3. the minimum capabilities needed for XDP;
4. a writable, access-controlled WAL directory when persistence is enabled;
5. a TLS/mTLS proxy for network-exposed management interfaces;
6. explicit monitoring of health, XDP state, and enforcement failures.

## Fuzzing

The repository includes property-based tests for the configuration and IPC protocol parsers. Continuous OSS-Fuzz integration is not currently enabled.