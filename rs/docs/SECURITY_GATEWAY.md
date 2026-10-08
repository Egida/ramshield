# RamShield Security Gateway — 0.6

RamShield 0.6 implements the host-local portions of the independent security review and makes deployment-dependent boundaries explicit. It does not claim that a host can defeat a physically saturated uplink or transparently inspect encrypted HTTP. The daemon remains first-principles and does not ingest public
community reputation feeds.

## 1. Autonomous observation

Proxy IPC is now one telemetry source, not the only source. On Linux,
`native_ingest.enabled=true` opens a Linux AF_PACKET TPACKET_V3 mmap ring on the protected
interface and emits bounded L3/L4 telemetry directly into the detection
channel. SYN and UDP packets are treated as high-signal; ordinary TCP traffic
is sampled under a hard events/sec ceiling.

This covers direct-to-origin SSH, DNS, custom TCP/UDP, and reverse-proxy bypass
at L3/L4. It does not decrypt TLS or reconstruct arbitrary TCP application
streams.

## 2. Kernel-first mitigation

The XDP program provides a runtime-configurable, per-CPU aggregate packet
budget plus bounded per-source SYN/UDP windows. Autonomous mode is qualified
for native driver XDP (`mode=drv`), not generic SKB XDP. The per-source map is an LRU
with a fixed 65,536-entry ceiling. The guard executes before userspace
EWMA/CUSUM processing.

For stateful handshake protection, `[synproxy]` installs Linux kernel
SYNPROXY/conntrack rules on selected TCP ports. This is the same layered model
used by the Linux/XDP SYNPROXY architecture: stateless XDP protection before
userspace and stateful handshake validation in the kernel. The nftables ruleset
also bounds per-source and global concurrent/new-connection rates to resist
Slowloris and TLS-handshake CPU exhaustion.

## 3. L7/WAF boundary

The bounded WAF parser accepts HTTP/1.0 and HTTP/1.1 request bytes supplied by
a trusted L7 telemetry producer. It parses the request line, headers,
Content-Length, and body before deterministic inspections for:

- SQL injection
- XSS
- path traversal
- command injection
- SSRF
- malformed/oversized requests

There is no backtracking regex engine. Request size, method length, target
length, header length and body size are bounded.

This is intentionally not advertised as transparent TLS WAF. Encrypted HTTP
requires a TLS termination point to supply plaintext request bytes. HTTP/2 and
HTTP/3 require protocol-aware stream parsers before payload inspection.

## 4. Upstream mitigation

When configured and uplink saturation is detected, RamShield can emit actual
ExaBGP text API commands through a restrictive FIFO. Supported actions are:

- FlowSpec discard for the configured protected prefix;
- RTBH route announcement using an explicitly configured blackhole community;
- withdrawal after recovery below the configured hysteresis threshold.

The FIFO is non-blocking. A dead BGP controller cannot stall detection.
ExaBGP remains the BGP state machine and credential boundary.

## 5. Security invariants

- No community threat-intelligence source is an enforcement authority.
- Local observations, trusted fleet signals, and operators remain the only
  enforcement provenance classes.
- WAL remains authoritative for committed blocks.
- XDP remains a projection plus first-line dataplane, never the durable state.
- Native observation has a bounded event budget.
- Kernel maps have fixed maximum cardinality.
- WAF parsing is bounded and deterministic.
- BGP integration cannot execute arbitrary shell commands.
- Public IPC requires HMAC credentials and a TLS/mTLS boundary.
