# RamShield Defense Model

RamShield is a sovereign, first-principles security guard. Enforcement evidence comes from traffic observed by the protected system, explicit operator policy, or authenticated RamShield fleet state.

## Defense ladder

```text
 upstream capacity
       |
 optional escalation (FlowSpec / RTBH / scrubber)
       |
 +-----v----------------+
 | XDP first-line guard |  <- cold-start packet protection
 | block maps           |
 | SYN/UDP/IP budgets   |
 +-----+----------------+
       |
   L3/L4 packet path
       |
 +-----v----------------+
 | userspace detection   |
 | EWMA / CUSUM / pulse  |
 | subnet / CGNAT        |
 | bounded L7 telemetry  |
 +-----+----------------+
       |
    WAL + state
       |
 trusted mesh only
```

XDP can protect the host before userspace has accumulated a sample window. Userspace remains authoritative for durable policy and complex detection. Upstream escalation is optional because a host cannot reclaim a saturated physical link.

## Autonomous packet guard

When `[autonomous].enabled=true` and XDP is enabled in native driver mode, the BPF program maintains fixed-window, per-CPU packet budgets without waking userspace. It can independently shed:

- TCP SYN packets above the configured SYN budget;
- UDP packets above the configured UDP budget;
- all parsed IP packets above the aggregate packet budget.

The guard is stateless and intentionally fails open on malformed packets. It is a cold-start safety valve, **not** a claim of SYN-cookie protection. The budgets are per CPU so the hot path avoids global atomics; qualification must account for the number of RX CPUs/queues.

## L7 boundary

RamShield is not yet a full WAF. L7 support consumes bounded metadata from a terminating application/proxy and reasons over normalized hashes, method/version, latency, bytes, and HTTP/2 stream counters.

The current native WAF boundary is HTTP/1.x only. A future native WAF must introduce bounded parsers for HTTP/1.1, HTTP/2, HTTP/3/QUIC, and WebSocket traffic, with normalization, body limits, parser timeouts, and explicit rule budgets. Raw bodies must never become an unbounded detection queue.

## Native packet ingestion boundary

XDP is the native L3/L4 observation and enforcement point. Application IPC remains an application telemetry interface, not the sole conceptual source of traffic evidence. Any future packet-to-userspace telemetry must use bounded/sampled ring-buffer delivery rather than copying raw packets into an unbounded queue.

## Uplink saturation

Host-local XDP cannot defeat a link that is already saturated upstream. RamShield may emit a vendor-neutral escalation signal when measured RX utilization approaches the configured capacity. A deployment can connect this to FlowSpec, RTBH, scrubbing, or provider automation without putting BGP credentials into the daemon.

## Management plane

HMAC authenticates IPC but does not encrypt it. Public management endpoints therefore require an external TLS/mTLS boundary until native TLS is implemented. A `tls_enabled` cookie/security flag must never be represented as wire encryption.

## Trusted mesh, not community intelligence

The RamShield mesh is a private authenticated operational-state channel. It is not a public reputation system.

RamShield deliberately has no authoritative:

- global IP reputation score;
- community banlist;
- CrowdSec-style shared reputation feed;
- external feed that can directly issue a block.

Trusted peers may share observations/state, but every such action is attributable to authenticated fleet evidence. The detector remains independent of any public reputation source.

## Capability closure matrix

| Capability | 0.6 foundation | Architectural rule |
|---|---|---|
| Host-local L3/L4 cold-start protection | **Implemented** | XDP packet/SYN/UDP budgets before userspace |
| Proxy/application telemetry | **Implemented** | bounded IPC; L7 metadata preserved into detection |
| Volumetric uplink protection | **Escalation boundary** | FlowSpec/RTBH/scrubbing remains upstream |
| CGNAT/shared subnet resilience | **Hardened** | 256K slots / 8 probes; projection failure never fabricates success |
| Full HTTP WAF | **Not claimed** | add bounded native parsers before OWASP-class rules |
| HTTP/2 Rapid Reset | **Telemetry detector** | native frame handling remains a later parser module |
| HTTP/3 / QUIC | **Not claimed** | UDP-aware guard exists; QUIC parser is separate work |
| WebSocket inspection | **Not claimed** | requires bounded L7 state machine |
| TLS JA3/JA4 | **Not claimed** | requires packet/TLS ClientHello observation |
| Native TLS/mTLS management plane | **Not claimed** | current public deployment requires an external TLS boundary |
| BGP FlowSpec / RTBH | **Adapter boundary** | daemon emits vendor-neutral escalation; provider integration owns BGP credentials |
| Multi-node consensus | **Not claimed** | authenticated fleet state is not a consensus protocol |
| Community threat intelligence | **Intentionally absent** | no public reputation source may authorize mitigation |

This matrix is part of the product contract: an unimplemented row must not be marketed as a completed security capability.
