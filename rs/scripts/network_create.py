#!/usr/bin/env python3
"""Create/destroy two isolated Linux network namespaces for RamShield traffic tests."""
from __future__ import annotations
import argparse, ipaddress, json, os, subprocess
from pathlib import Path

BRIDGE = "rs_test_br"
BASE = ipaddress.ip_network("192.0.2.0/24")

def build_plan(count: int, family: str, prefixlen: int, hosts_per_network: int, seed: int) -> list[dict]:
    if count < 1 or hosts_per_network < 1 or family not in {"v4", "v6", "dual"}:
        raise ValueError("invalid fixture dimensions")
    if family in {"v4", "dual"} and prefixlen != 24:
        raise ValueError("IPv4 test fixtures require /24")
    if family == "v6" and prefixlen != 64:
        raise ValueError("IPv6 test fixtures require /64")
    out = []
    for i in range(count):
        net = ipaddress.ip_network(f"192.0.{2 + i}.0/24") if family != "v6" else ipaddress.ip_network(f"2001:db8:{seed + i}::/64")
        hosts = [{"ip": str(net.network_address + (j % (net.num_addresses - 2)) + 1), "events": 1} for j in range(hosts_per_network)]
        out.append({"network": str(net), "family": 4 if net.version == 4 else 6, "hosts": hosts})
    return out

def ramshield_overlay(interface: str, ipc_port: str, dashboard_port: str) -> dict:
    return {"xdp": {"enabled": True, "interface": interface, "mode": "skb"}, "ipc": {"tcp_addr": f"127.0.0.1:{ipc_port}"}, "dashboard": {"enabled": True, "http_addr": f"127.0.0.1:{dashboard_port}"}}

def run(cmd: list[str], check: bool = True) -> subprocess.CompletedProcess:
    return subprocess.run(cmd, check=check, capture_output=True, text=True)

def create(count: int) -> dict:
    if os.geteuid() != 0:
        raise PermissionError("network_create create requires root")
    run(["ip", "link", "add", BRIDGE, "type", "bridge"], check=False)
    run(["ip", "link", "set", BRIDGE, "up"])
    envs = []
    for i in range(count):
        ns, host, peer = f"rs_test_{i}", f"rs_veth_h{i}", f"rs_veth_n{i}"
        run(["ip", "netns", "add", ns])
        run(["ip", "link", "add", host, "type", "veth", "peer", "name", peer])
        run(["ip", "link", "set", peer, "netns", ns])
        run(["ip", "link", "set", host, "master", BRIDGE])
        run(["ip", "link", "set", host, "up"])
        run(["ip", "netns", "exec", ns, "ip", "link", "set", "lo", "up"])
        run(["ip", "netns", "exec", ns, "ip", "addr", "add", f"192.0.2.{100 + i}/24", "dev", peer])
        run(["ip", "netns", "exec", ns, "ip", "link", "set", peer, "up"])
        envs.append({"namespace": ns, "address": f"192.0.2.{100 + i}"})
    return {"schema": "ramshield.network.v1", "bridge": BRIDGE, "environments": envs}

def destroy(count: int) -> None:
    if os.geteuid() != 0:
        raise PermissionError("network_create destroy requires root")
    for i in range(count):
        run(["ip", "link", "del", f"rs_veth_h{i}"], check=False)
        run(["ip", "netns", "del", f"rs_test_{i}"], check=False)
    run(["ip", "link", "del", BRIDGE], check=False)

def main() -> int:
    p = argparse.ArgumentParser()
    p.add_argument("action", choices=("plan", "create", "destroy"))
    p.add_argument("--count", type=int, default=2)
    p.add_argument("--output", default="-")
    args = p.parse_args()
    try:
        result = build_plan(args.count, "v4", 24, 32, 7) if args.action == "plan" else (create(args.count) if args.action == "create" else (destroy(args.count) or {"destroyed": args.count}))
    except (ValueError, PermissionError, subprocess.CalledProcessError) as e:
        print(f"network_create: {e}")
        return 2
    text = json.dumps(result, indent=2, sort_keys=True)
    if args.output == "-": print(text)
    else: Path(args.output).write_text(text + "\n")
    return 0

if __name__ == "__main__": raise SystemExit(main())
