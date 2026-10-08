# RamShield on Kubernetes

RamShield has two intentionally different Kubernetes profiles. The ordinary
server Deployment keeps control surfaces on loopback and runs without XDP.
The node-guard DaemonSet uses host networking and the host BPF filesystem when
XDP enforcement is required.

## Manifests

| File | Purpose |
| --- | --- |
| `namespace.yaml` | Namespace |
| `configmap.yaml` | Loopback-only server configuration; XDP disabled |
| `deployment.yaml` | Single-replica server; durable WAL on a `ReadWriteOnce` PVC |
| `configmap-node.yaml` | Node-guard configuration |
| `daemonset.yaml` | Host-network XDP node guard |
| `rbac.yaml` | Service account / minimal RBAC |
| `networkpolicy.yaml` | Network restrictions |
| `poddisruptionbudget.yaml` | Single-replica availability policy |

There is deliberately **no ClusterIP service for IPC or the dashboard**. Use
`kubectl port-forward` or an explicitly configured authenticated TLS/mTLS
proxy.

## Server profile

The ordinary Deployment uses loopback-only IPC/dashboard, XDP disabled, one
replica because the WAL is local to the pod, a read-only root filesystem, and
no Linux capabilities.

```bash
kubectl -n ramshield port-forward pod/<ramshield-pod> 9999:9999
```

The dashboard password hash and IPC authentication key come from the
`ramshield-admin` Secret.

## Node-guard / XDP profile

The DaemonSet uses `hostNetwork: true`, the host `/sys/fs/bpf`, memory-backed
`/dev/shm`, a host WAL directory, non-root UID/GID 65532, RuntimeDefault
seccomp, privilege escalation disabled, and only `NET_ADMIN`, `BPF`, `PERFMON`,
and `NET_RAW` capabilities. `BPF` is the Kubernetes capability name
corresponding to Linux `CAP_BPF`. The node guard uses the dedicated
`Containerfile.node-guard` image because SYNPROXY/RSS setup requires `nft`,
`sysctl`, and `ethtool`; the ordinary server image remains distroless.

XDP remains kernel/driver/NIC dependent and must be qualified on target nodes.

## Build and deploy

```bash
docker build --build-arg RAMSHIELD_VERSION=0.6.0 -t ghcr.io/grep999/ramshield:0.6.0 .

# Build the host-network node guard after producing target/release/ramshield.
docker build -f Containerfile.node-guard -t ghcr.io/grep999/ramshield:0.6.0-node .

docker push ghcr.io/grep999/ramshield:0.6.0
docker push ghcr.io/grep999/ramshield:0.6.0-node
kubectl apply -f deploy/k8s/
```

Do not deploy `latest`; release identity comes from the tagged release.

## Storage model

The server Deployment is intentionally single-replica. Each process owns its
WAL and in-memory enforcement state. Multiple replicas without a shared WAL
and coordinated authoritative state would create independent enforcement
truths.

## Health

`/healthz` is the liveness/readiness contract. With XDP configured and no
allowed fallback, attach failure or a stale projection affects health according
to the enforcement configuration. `/metrics` is Prometheus text exposition,
not JSON.

## Autonomous host defense

The node-guard profile now enables three independent host-level defenses:

1. XDP per-CPU packet/SYN/UDP budgets for cold-start flood control.
2. Native AF_PACKET L3/L4 observation so direct SSH/DNS/custom-port attacks do not depend on proxy IPC.
3. Linux kernel SYNPROXY is enabled in the node-guard profile; its dedicated host-tooling image supplies `nft` and `sysctl`.

`NET_RAW` is required for AF_PACKET. `NET_ADMIN` is required for netfilter/XDP setup.
Qualify the configured PPS thresholds and SYNPROXY ports on each node class before rollout.

The ordinary Deployment intentionally leaves these host-network defenses disabled.
