# RamShield 0.6 — Security Gateway Feature Pack

This is the consolidated implementation for the 0.6 security-gateway hardening pass.
It is intentionally one feature pack rather than a collection of independent patches.

## Threats covered

| Threat | Implementation | State |
|---|---|---|
| Proxy bypass / direct scans | AF_PACKET TPACKET_V3 + PACKET_FANOUT_HASH | implemented |
| SYN flood | XDP autonomous SYN budget + nft SYNPROXY | implemented; runtime qualification required |
| Slowloris / pre-request starvation | SYNPROXY + per-source/global new-connection limits | implemented; runtime qualification required |
| Asymmetric L7 cost | route hash + latency + configurable cost_weight + effective-RPS ceiling | implemented |
| HTTP request smuggling | bounded HTTP parser, conflicting CL/TE rejection | implemented |
| SSRF false positives | Host header excluded from SSRF search | implemented |
| IPv4 fragments | fail-closed XDP fragment handling | implemented |
| IPv6 fragments/extensions | bounded extension walk; Fragment/ESP fail closed | implemented |
| VXLAN | bounded inner Ethernet/IP recovery | implemented |
| Geneve | version/options-aware bounded offset | implemented |
| GRE | bounded optional-field offset; IPv4/IPv6 inner support | implemented |
| Cloud/LB shared source identity | trusted overlay LPM maps; do not ban outer identity without tenant identity | implemented |
| RSS concentration | ethtool RSS rebalance, optionally fail-closed | implemented; NIC qualification required |
| Physical uplink saturation | byte-rate monitor + FlowSpec/RTBH/webhook escalation | implemented; upstream qualification required |
| IPv6 prefix hopping | bounded reverse subnet index | implemented |
| Mesh operator race | operator suppression over delayed fleet re-add | implemented; eventual consistency only |
| Enforcement burst stalls | receive batching, coalescing, TTL yielding | improved; not yet parallel kernel-map batching |
| ARM64 SHM race | acquire/release seqlock publication | implemented; ARM hardware qualification required |

## Product boundary

RamShield does not depend on community/global threat intelligence. Detection is derived
from local observation, authenticated fleet signals, and explicit operator evidence.

It is not a Cloud/WAF replacement and cannot repair a physical carrier link that is
already saturated. Upstream mitigation exists specifically for that boundary.

## Dataplane hierarchy

```text
NIC/RSS
  |
  +--> native XDP
  |      - blocklists / CIDRs
  |      - fragments / malformed packet policy
  |      - autonomous SYN/UDP/packet budgets
  |      - overlay identity recovery
  |
  +--> nftables SYNPROXY
  |      - handshake admission
  |      - per-source/global new connection controls
  |
  +--> TPACKET_V3 observation
  |      - direct ingress telemetry
  |      - overlay-aware source identity
  |
  +--> application telemetry
         - route identity
         - latency
         - HTTP/2 stream/reset data
         - weighted effective request cost
```

## Deployment profiles

### Bare-metal/systemd

Use the normal release binary. Configure:

- native XDP (`drv`)
- native ingress capture
- RSS rebalance
- SYNPROXY
- real IPC authentication keys
- upstream link capacity if FlowSpec/RTBH is desired

### Kubernetes

There are two deliberately separate pods:

- `Deployment`: ordinary application/control-plane server, pod networking, no host capabilities.
- `DaemonSet`: host-network node guard, XDP, AF_PACKET, RSS, SYNPROXY, host WAL.

The node guard uses `Containerfile.node-guard` because the distroless application image
cannot execute `nft`, `sysctl`, or `ethtool`.

## Configuration invariants

1. Autonomous protection requires native driver XDP.
2. Production autonomous deployments should require RSS rebalance.
3. Trusted overlay CIDRs must identify infrastructure addresses, not tenant ranges.
4. Public IPC binds require HMAC authentication and a TLS/mTLS boundary.
5. SYNPROXY MSS/window-scale values must match the protected service path.
6. Upstream BGP configuration must specify a non-/0 protected prefix.
7. RTBH requires an explicit ASN:community value.

## Qualification gate

Run:

```bash
./scripts/verify_maturity.sh
```

Then perform live Linux qualification:

```text
native ingress flood
SYN flood
Slowloris / incomplete TLS
IPv4 fragments
IPv6 extension chains/fragments
VXLAN/Geneve/GRE
RSS concentration
50k unique enforcement decisions
50k TTL expirations
IPv6 /64 hopping
mesh delayed re-add/operator suppression
FlowSpec announce/withdraw
RTBH announce/withdraw
```

The source tree must not be called production-qualified until the runtime gates pass.
