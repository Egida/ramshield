#!/usr/bin/env python3
"""Deterministic unique-network traffic generator for local RamShield testing.

Emits valid IPC request frames only; addresses are synthetic RFC1918/RFC3849.
No packets leave the host. Use the output as a fixture or pipe it to a client
that connects to a RamShield IPC listener you own.
"""
from __future__ import annotations

import argparse
import hashlib
import ipaddress
import json
import random
import socket
import sys
from dataclasses import dataclass
from typing import Iterator

V4_NETWORKS = 1 << 24  # 10.0.0.0/8 as /24 networks
V6_NETWORKS = 1 << 16  # 2001:db8:1000::/48 as /64 networks


@dataclass(frozen=True)
class Network:
    network: str
    family: int
    phase: str


class UniqueNetworkAllocator:
    """Collision-free affine permutation over a bounded network address space."""

    def __init__(self, seed: int = 0, family: str = "dual") -> None:
        if family not in {"v4", "v6", "dual"}:
            raise ValueError("family must be v4, v6, or dual")
        digest = hashlib.blake2b(str(seed).encode(), digest_size=16).digest()
        self._offset4 = int.from_bytes(digest[:4], "big") % V4_NETWORKS
        self._offset6 = int.from_bytes(digest[4:6], "big") % V6_NETWORKS
        self._a4 = ((int.from_bytes(digest[6:10], "big") | 1) % V4_NETWORKS) or 1
        self._a6 = ((int.from_bytes(digest[10:12], "big") | 1) % V6_NETWORKS) or 1
        self.family = family
        self._seen: set[str] = set()

    def _v4(self, index: int) -> ipaddress.IPv4Network:
        slot = (self._a4 * index + self._offset4) % V4_NETWORKS
        return ipaddress.ip_network(f"10.{slot >> 16}.{(slot >> 8) & 255}.0/24")

    def _v6(self, index: int) -> ipaddress.IPv6Network:
        slot = (self._a6 * index + self._offset6) % V6_NETWORKS
        return ipaddress.ip_network(f"2001:db8:1000:{slot:x}::/64")

    def networks(self, count: int) -> Iterator[Network]:
        if count < 1:
            raise ValueError("count must be positive")
        for index in range(count):
            families = ("v4", "v6") if self.family == "dual" else (self.family,)
            for family in families:
                net = self._v4(index) if family == "v4" else self._v6(index)
                text = str(net)
                if text in self._seen:
                    raise AssertionError(f"network collision: {text}")
                self._seen.add(text)
                yield Network(text, 4 if family == "v4" else 6, _phase(index))


def _phase(index: int) -> str:
    return ("hotspot", "subnet_surge", "rotation", "botnet", "cooldown")[index % 5]


def _host(net: ipaddress._BaseNetwork, offset: int) -> str:
    # Keep host addresses valid for both IPv4 and IPv6; never emit network/broadcast.
    return str(net.network_address + (offset % max(net.num_addresses - 2, 1)) + 1)


def events(networks: list[Network], per_network: int, seed: int) -> Iterator[dict]:
    rng = random.Random(seed)
    for nindex, item in enumerate(networks):
        net = ipaddress.ip_network(item.network)
        for event_index in range(per_network):
            host_offset = (event_index * 17 + nindex * 13 + rng.randrange(1, 32))
            status = {"hotspot": 200, "subnet_surge": 503, "rotation": 404,
                      "botnet": 429, "cooldown": 200}[item.phase]
            proto = {"hotspot": 0x1100, "subnet_surge": 0x3000,
                     "rotation": 0x5000, "botnet": 0x7000,
                     "cooldown": 0x1000}[item.phase]
            yield {"ip": _host(net, host_offset), "bytes": rng.randint(128, 8192),
                   "status_code": status, "proto_fp": proto | rng.randrange(256)}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--networks", type=int, default=32)
    parser.add_argument("--events-per-network", type=int, default=32)
    parser.add_argument("--seed", type=int, default=20260917)
    parser.add_argument("--family", choices=("v4", "v6", "dual"), default="dual")
    parser.add_argument("--batch-size", type=int, default=256)
    parser.add_argument("--metadata", action="store_true")
    parser.add_argument("--send", action="store_true", help="send frames to --host/--port")
    parser.add_argument("--host", default="127.0.0.1")
    parser.add_argument("--port", type=int, default=7890)
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    if args.self_test:
        for family in ("v4", "v6", "dual"):
            plan = list(UniqueNetworkAllocator(7, family).networks(128))
            assert len({item.network for item in plan}) == len(plan)
            assert all(ipaddress.ip_network(item.network).prefixlen in (24, 64) for item in plan)
        sample = list(events(list(UniqueNetworkAllocator(7, "dual").networks(4)), 8, 7))
        assert len(sample) == 64
        assert all(ipaddress.ip_address(item["ip"]) for item in sample)
        print("OK: unique v4/v6 networks, valid hosts, IPC frames")
        return 0
    allocator = UniqueNetworkAllocator(args.seed, args.family)
    allocated = list(allocator.networks(args.networks))
    if args.metadata:
        for item in allocated:
            print(json.dumps({"type": "network", "network": item.network,
                              "family": item.family, "phase": item.phase}))
    frames: list[dict] = []
    batch: list[dict] = []
    for event in events(allocated, args.events_per_network, args.seed):
        batch.append(event)
        if len(batch) == args.batch_size:
            frames.append({"type": "report_connections", "events": batch})
            batch = []
    if batch:
        frames.append({"type": "report_connections", "events": batch})
    if not args.send:
        for frame in frames:
            print(json.dumps(frame))
        return 0
    sent = failed = total_events = 0
    for frame in frames:
        try:
            with socket.create_connection((args.host, args.port), timeout=3) as sock:
                sock.sendall((json.dumps(frame) + "\n").encode())
                sock.settimeout(3)
                sock.recv(65536)
            sent += 1
            total_events += len(frame["events"])
        except (OSError, TimeoutError) as exc:
            failed += 1
            print(f"send failure frame={sent + failed}: {exc}", file=sys.stderr)
    print(f"sent_frames={sent} failed_frames={failed} events={total_events}")
    return 1 if failed else 0


if __name__ == "__main__":
    raise SystemExit(main())


# Runnable invariant check: python3 scripts/unique_network_generator.py --self-test
