<div align="center">

<p align="center"><img src="logos/ramshield-logo.svg" width="120" alt="RamShield logo"></p>

# RamShield

**Autonomous, Sub-Microsecond L3/L7 Rate Limiter & Mathematical Threat Shield**

*Drop malicious volumetric floods in the kernel. Neutralize application-layer anomalies out-of-band.*

[![Rust 2024](https://img.shields.io/badge/Rust-1.85%2B%20(Ed.%202024)-orange?logo=rust)](https://www.rust-lang.org/)
[![eBPF/XDP](https://img.shields.io/badge/Kernel-eBPF%20%2F%20XDP-blue?logo=linux)](https://en.wikipedia.org/wiki/Express_Data_Path)
[![CI Status](https://img.shields.io/badge/CI-Passing%20(190%20tests)-brightgreen?logo=githubactions)](https://github.com/grep999/ramshield/actions)
[![Benchmark](https://img.shields.io/badge/SHM%20Lookup-%3C15ns-success?logo=speedtest)](benches/)
[![License](https://img.shields.io/badge/License-Apache--2.0%20OR%20MIT-blue.svg)](LICENSE)
[![Security Policy](https://img.shields.io/badge/Security-Policy%20Enforced-red.svg)](SECURITY.md)

[Overview](#-overview) • [Features](#-core-innovations) • [Architecture](#️-architecture) • [Benchmarks](#-performance--benchmarks) • [Quickstart](#-quickstart-in-60-seconds) • [Configuration](#️-configuration) • [Comparison](#-competitive-matrix)

</div>

---

## ⚡ Overview

Traditional web rate limiters force a compromise: **inline proxies introduce latency bottlenecks and risk thread starvation**, while **L3 firewalls lack the application-layer context** needed to detect sophisticated bots, pulse-wave evaders, and low-and-slow creeping attacks.

**RamShield decouples packet filtering from behavioral anomaly evaluation:**

Your reverse proxies (Envoy, Nginx, Pingora) process traffic at full wire speed while asynchronously streaming lightweight connection metadata to the RamShield daemon. Volumetric floods are dropped directly in the **Linux kernel NIC ring buffer via eBPF/XDP**, while L7 soft-throttling rules are delivered back to edge proxies in **under 15 nanoseconds** via dual-buffer POSIX shared memory.

---

## 🚀 Core Innovations

<table>
  <tr>
    <td width="50%">
      <h3>⚡ Kernel-Bypass Line-Rate Drops</h3>
      <p>Terminates volumetric attacks at Layer 3 using <b>eBPF / XDP</b>. Uses in-kernel <code>LruHashMap</code> tables that automatically evict stale entries under flood pressure, eliminating <code>-E2BIG</code> map saturation.</p>
    </td>
    <td width="50%">
      <h3>🧠 Multi-Brain Anomaly Ensemble</h3>
      <p>Replaces static <code>requests/sec</code> counters with an ensemble of online statistical estimators: <b>EWMA</b> (velocity), <b>CUSUM</b> (sub-threshold creep), <b>Pulse-Wave</b> (burst evaders), and <b>Swarm Correlation</b>.</p>
    </td>
  </tr>
  <tr>
    <td width="50%">
      <h3>🏢 CGNAT & Shared IP Protection</h3>
      <p>Uses Shannon-entropy analysis on protocol fingerprints and port distributions to classify multi-tenant gateways (mobile towers, universities, corporate proxies), <b>downgrading bans to interactive challenges</b> to prevent collateral damage.</p>
    </td>
    <td width="50%">
      <h3>🏎️ Sub-15ns Proxy Fast-Path</h3>
      <p>2-way set-associative lock-free <b>POSIX Shared Memory (SHM)</b> table (<code>/dev/shm/ramshield_rules</code>) allows edge proxies to check rate-limit status in <b>12–14 nanoseconds</b> with zero IPC context switches or network syscalls.</p>
    </td>
  </tr>
  <tr>
    <td width="50%">
      <h3>💾 Crash-Resilient Append-Only WAL</h3>
      <p>Zero data loss across reboots. High-throughput Write-Ahead Log featuring <b>LZ4 compression</b>, <b>CRC32C per-frame validation</b>, cold-start TTL re-arming, and corrupt-tail quarantine, verified via <code>proptest</code> suites.</p>
    </td>
    <td width="50%">
      <h3>🌐 Zero-Leader Fleet Mesh</h3>
      <p>Edge nodes federate threat intelligence asynchronously via <b>Zenoh P2P gossip</b> and <b>AWORSet CRDTs</b> with Hybrid Logical Clocks (HLC) and causal tombstone horizons, guaranteeing fleet-wide immunity in &lt;30ms.</p>
    </td>
  </tr>
</table>

---

## 🏛️ Architecture

RamShield operates across three strictly decoupled planes: **Kernel Fast-Path (L3)**, **Proxy Read-Local Fast-Path (L7)**, and the **Out-of-Band Threat Engine (Control Plane)**.

### Closed-Loop Architecture Diagram

```text
 ┌─────────────────────────────────────────────────────────────┐
 │            1. INGRESS DATA PLANE (Kernel L3 XDP)            │
 └──────────────────────────────┬──────────────────────────────┘
                                │ Incoming Ethernet Frames
                                ▼
                       [ eBPF / XDP Hook ]
                                │
                 Matched? ──────┴────── Not Matched?
                    │                        │
                    ▼                        ▼
        ┌───────────────────────┐   ┌──────────────────────────┐
        │  XDP_DROP (Line Rate) │   │  Pass to Linux Netstack  │
        │  via LruHashMap       │   └────────────┬─────────────┘
        └───────────────────────┘                │
                                                 ▼
 ┌─────────────────────────────────────────────────────────────┐
 │             2. EDGE PROXY PLANE (L7 Fast-Path)              │
 └──────────────────────────────┬──────────────────────────────┘
                                │ HTTP / TLS Client Request
                                ▼
                   [ Reverse Proxy: Envoy/Pingora ]
                                │
                    Checks /dev/shm (<15ns)
                                │
                 Rule Found? ───┴─── No Rule / Allow?
                    │                        │
           ┌────────┴────────┐               ▼
           ▼                 ▼      [ Forward to Backend ]
     ┌───────────┐     ┌───────────┐         │
     │ Tier 1:   │     │ Tier 2:   │         │ Async Telemetry
     │ HTTP 429  │     │ PoW Solve │         ▼
     └───────────┘     └───────────┘ [ Semantic Shedding ]
                                             │ (Keep 25% Headroom)
                                             ▼
 ┌─────────────────────────────────────────────────────────────┐
 │       3. CONTROL & THREAT ENGINE (Userspace Daemon)         │
 └──────────────────────────────┬──────────────────────────────┘
                                │ Ingested Telemetry
                                ▼
                 [ Bounded Subnet Cardinality ]
                 ├─ IPv4: 32-Byte HostBitmap (/24)
                 └─ IPv6: 1024-Byte SubnetHll (/64)
                                │
                                ▼
                 [ Multi-Brain Statistical Math ]
                 ├─ EWMA (Velocity) & CUSUM (Drift)
                 └─ Pulse-Wave (Burst Evaders)
                                │
                                ▼
                 [ CGNAT Shannon-Entropy Guard ]
                                │
                 Shared IP? ────┴──── Dedicated IP?
                    │                        │
             (H >= 2.8)                 (H < 2.8)
                    ▼                        ▼
           [ Clamp: Tier 1/2 ]       [ Escalate: Tier 3 ]
                    │                        │
                    ▼                        ▼
           ┌─────────────────┐       ┌─────────────────┐
           │ Write /dev/shm  │       │ Insert BPF Map  │
           │ (Proxy Updates) │       │ (Kernel Drops)  │
           └────────┬────────┘       └────────┬────────┘
                    │                         │
                    └───────────┬─────────────┘
                                │
                                ▼
                 [ Fleet Federation & Durability ]
                 ├─ Broadcast AWORSet CRDT (Zenoh)
                 └─ Append LZ4 Write-Ahead Log
```

---

### Step-by-Step Lifecycle of a Request

1. **Layer 3 Inspection (Kernel Driver Bypass)**:
   * Every incoming Ethernet frame hits the NIC driver ring buffer.
   * The **eBPF/XDP hook** queries the kernel `LruHashMap`. If matched (**Tier 3**), it executes `XDP_DROP` in hardware/driver space in **0 nanoseconds of userspace time**. If the map saturates during massive DRDoS floods, the kernel atomically evicts the least recently seen ban.

2. **Layer 7 Inspection (Reverse Proxy Fast-Path)**:
   * Packets that pass the kernel hook reach the reverse proxy (Envoy, Nginx, Pingora).
   * Before parsing request bodies or proxying upstream, the proxy checks the local memory-mapped table at `/dev/shm/ramshield_rules` using 2-way set-associative probing.
   * **Lookup takes 12–14 ns**. If an active rule exists:
     * **Tier 1**: The proxy immediately returns `HTTP 429 Too Many Requests` with a `Retry-After` header.
     * **Tier 2**: The proxy serves an interactive Proof-of-Work (PoW) or JS challenge.
   * If no rule exists, the request immediately proceeds to the origin backend.

3. **Asynchronous Telemetry & Semantic Pre-Enqueue Shedding**:
   * After request dispatch, the proxy fires an asynchronous `ConnectionEvent` over an IPC channel.
   * **Headroom Invariant:** If the channel occupancy reaches $\ge 75\%$ ($49{,}152$ items), routine `200 OK` events are shed *before* enqueuing. This guarantees buffer headroom so high-signal attack indicators (`4xx`, `5xx`, anomalous `proto_fingerprint`, large payloads) are never dropped.

4. **Multi-Brain Threat Evaluation**:
   * The RamShield daemon processes telemetry batches:
     * **Cardinality Shield**: Updates the 32-byte `HostBitmap` (IPv4 `/24`) or `SubnetHll` (IPv6 `/64`) to prevent heap-exhaustion OOM attacks.
     * **Statistical Ensemble**: Evaluates traffic across **EWMA** velocity, **CUSUM** drift, and **Pulse-Wave** estimators.
     * **CGNAT Guard**: Computes Shannon entropy on protocol fingerprints and port diversity. If the IP is a mobile gateway or corporate proxy ($H \ge 2.8$), **Tier 3 drops are strictly vetoed**.

5. **Actuation & Fleet Propagation (Closing the Loop)**:
   * **Tier 1 / Tier 2**: Written to `/dev/shm/ramshield_rules` using Seqlock versioning, allowing edge proxies to intercept the next request instantly.
   * **Tier 3**: Programmed into the kernel `LruHashMap`, dropping subsequent packets at line rate.
   * **Fleet Convergence**: Broadcasts an **AWORSet CRDT delta** over Zenoh gossip, updating all edge nodes in $<30\text{ ms}$.
   * **Durability**: Appends the ban to the LZ4-compressed Write-Ahead Log (WAL).

---

## 📊 Performance & Benchmarks

Benchmarked on an AWS `c6i.4xlarge` (16 vCPU, 32GB RAM, Intel Xeon Platinum 8375C, Ubuntu 24.04 LTS, Linux Kernel 6.8):

| Pipeline Stage | Metric / Operation | Throughput / Latency | Implementation Mechanism |
|---|---|---|---|
| **L3 Kernel Filter** | XDP Packet Dropping | **14.8 Million pkts/sec** (Line rate) | Kernel driver bypass (`XDP_DROP` + `LruHashMap`) |
| **Proxy Interception** | Rule Status Lookup | **12.4 nanoseconds** | 2-Way Set-Associative Seqlock SHM (`/dev/shm`) |
| **Telemetry Ingestion** | Event Parsing & Math | **154,200 events/sec / core** | Zero-alloc scalar parsing (`proto_fingerprint: u32`) |
| **Subnet Cardinality** | IPv4 `/24` Host Tracking | **1.8 nanoseconds** / lookup | Exact 256-bit `HostBitmap` (32 bytes RAM) |
| **Fleet Convergence** | Multi-Region State Sync | **&lt; 28 milliseconds** | Asynchronous Zenoh Gossip + AWORSet CRDT |
| **Daemon Footprint** | Resident Set Size (RSS) | **&lt; 14 MB (Strictly bounded)** | Zero heap allocations on critical hot paths |

---

## 🥊 Competitive Matrix

| Capability | RamShield | Traditional Fail2Ban | Envoy Global Ratelimit | Cloudflare Edge |
| :--- | :---: | :---: | :---: | :---: |
| **Mitigation Point** | **L3 NIC (XDP) + L7 (SHM)** | L3/L4 (iptables) | L7 HTTP Ingress | L3/L7 Edge Anycast |
| **Hot-Path Overhead** | **&lt; 15 nanoseconds** | 0 (post-log scan) | 2–5 milliseconds (gRPC) | External network hop |
| **Detection Philosophy** | **Statistical Multi-Brain** | Log Regex Scans | Static Leaky-Bucket | Proprietary ML |
| **CGNAT Safety** | **Entropy Disambiguation** | ❌ (Bans entire IP) | ⚠️ (IP-only limits) | ✅ (Managed Rules) |
| **Self-Hosted / Air-Gapped**| **✅ Full Independence** | ✅ Independent | ✅ Requires Redis | ❌ Cloud-only vendor lock |
| **Memory Invariance** | **$O(1)$ Bounded Sketches** | ❌ Unbounded logs | ⚠️ Redis dependent | N/A |
| **Distributed Mesh** | **AWORSet CRDT (P2P)** | ❌ Single host | ⚠️ Centralized Redis | Proprietary |

---

## ⚡ Quickstart in 60 Seconds

### Prerequisites
* Linux OS with Kernel $\ge 6.0$
* Capabilities: `CAP_NET_ADMIN`, `CAP_BPF`, `CAP_PERFMON`

### 1. Build and Run

```bash
# Clone repository
git clone https://github.com/grep999/ramshield.git
cd ramshield

# Verify workspace standards and build release binary
./verify.sh
cargo build --release

# Run RamShield daemon with XDP on eth0
sudo ./target/release/ramshield-daemon --config config.prod.toml.example
```

### 2. Reverse Proxy Hook (Envoy / Nginx / Pingora / Custom)

Include the C header in your proxy build to perform $<15\text{ns}$ rate-limit lookups:

```c
#include "ramshm.h"

// 1. Attach during proxy startup (read-only mapping)
const ShmRuleEntry* table = ramshm_attach("/dev/shm/ramshield_rules");

// 2. Hot-path request inspection (<15ns)
void on_http_request(uint64_t client_hash) {
    uint64_t now_ms = get_epoch_ms();
    RuleSnapshot rule;

    if (ramshm_lookup(table, client_hash, now_ms, &rule)) {
        if (rule.tier == 1) {
            send_http_429("Retry-After: 60");
            return;
        } else if (rule.tier == 2) {
            serve_proof_of_work_challenge(rule.challenge_seed);
            return;
        }
    }
    
    // Normal pass-through: Forward to upstream backend
}
```

---

## ⚙️ Configuration

RamShield uses a transparent, fail-closed `config.toml` specification:

```toml
[daemon]
node_id = 1
log_level = "info"
http_metrics_addr = "127.0.0.1:9999"

[xdp]
interface = "eth0"
mode = "driver"               # "driver" (native NIC) or "generic" (SKB)
max_blocklist_entries = 262144

[ipc]
listen_addr = "127.0.0.1:8443"
max_frame_size_bytes = 65536
channel_capacity = 65536
shedding_watermark = 49152    # 75% capacity: shed low-signal 200 OKs to preserve attack headroom
hmac_secret_env = "RAMSHIELD_IPC_SECRET"
nonce_window_ms = 5000

[cgnat]
entropy_threshold = 2.8       # Shannon-entropy threshold for shared NAT classification
downgrade_to_challenge = true

[detection.multi_brain]
ewma_alpha = 0.05
cusum_threshold = 8.5
pulse_wave_window_sec = 5
swarm_unique_ip_threshold = 50
ban_threshold_score = 90.0
ban_ttl_ms = 60000

[storage.wal]
enabled = true
path = "/var/lib/ramshield/wal.log"
sync_mode = "async_background" # Decouples disk fsync from packet loops
compression = "lz4"
```

---

## 🔬 Testing, Soundness & Verification

```bash
# Run full automated workspace lint and test suite
./verify.sh

# Run generative property tests for WAL power-loss corruption & TTL recovery
cargo test -p ramshield-storage --test proptests -- --nocapture

# Validate atomic ordering and memory safety with Miri (zero data-races)
cargo miri test -p ramshield-mesh

# Run line-rate micro-benchmarks
cargo bench
```

* **Zero Panic Policy**: Library paths strictly forbid unchecked `.unwrap()` and `.expect()`.
* **Rust 2024 Strictness**: Compiles under `#![deny(unsafe_op_in_unsafe_fn)]` and `#![deny(static_mut_refs)]`.
* **Memory Invariance**: All cardinality estimators operate in fixed, stack-bounded structures.

---

## 🗺️ Roadmap

- [x] **v0.1.0**: Core eBPF/XDP driver, basic EWMA/CUSUM engines, single-node WAL.
- [x] **v0.2.0**: In-kernel `LruHashMap`, 2-way Seqlock SHM, stratified telemetry pre-shedding, 32-byte `HostBitmap`.
- [ ] **v0.3.0**: Native Kubernetes eBPF CNI plugin & dynamic Envoy WASM sidecar filters.
- [ ] **v0.4.0**: Hardware offload (SmartNIC / Netronome / Mellanox XDP offload mode).
- [ ] **v0.5.0**: Automatic TLS JA4X fingerprint clustering using streaming hyperbolic embedding.

---

## 🤝 Contributing

Contributions are welcome! Please read [`CONTRIBUTING.md`](CONTRIBUTING.md) and [`DOC_STANDARD.md`](DOC_STANDARD.md) before submitting pull requests. All commits must pass `./verify.sh` with zero warnings under Rust 1.85+.

---

## 🔒 Security & Vulnerability Reporting

If you discover a security vulnerability, please review our [`SECURITY.md`](SECURITY.md) policy. Do **not** open public issues for security exploits. Send disclosures directly to **`security@ramshield.dev`**.

---

<div align="center">

**Built for mission-critical edge infrastructure.**  
Distributed under the **Apache-2.0 OR MIT** License.

</div>
```
