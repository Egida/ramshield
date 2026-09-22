#!/usr/bin/env python3
"""Read-only network status and canonical address formatting for RamShield tests."""
from __future__ import annotations
import argparse, ipaddress, json, subprocess
from typing import Any

SCHEMA = "ramshield.network.v1"

def canonical_network(address: str, prefixlen: int) -> str:
    return str(ipaddress.ip_network(f"{address}/{prefixlen}", strict=False))

def _ip_json(args: list[str]) -> list[dict[str, Any]]:
    try:
        p = subprocess.run(["ip", "-j", *args], check=True, capture_output=True, text=True)
    except FileNotFoundError as e:
        raise RuntimeError("ip command unavailable") from e
    return json.loads(p.stdout or "[]")

def collect(interface: str | None = None) -> dict[str, Any]:
    links = _ip_json(["link", "show"])
    addrs = _ip_json(["addr", "show"])
    by_name: dict[str, dict[str, Any]] = {}
    for link in links:
        name = link.get("ifname")
        if not name or (interface and name != interface):
            continue
        by_name[name] = {
            "name": name, "index": link.get("ifindex"),
            "oper_state": link.get("operstate", "UNKNOWN"),
            "admin_up": "UP" in link.get("flags", []),
            "mtu": link.get("mtu"), "mac": link.get("address"), "addresses": [],
        }
    for item in addrs:
        name = item.get("ifname")
        if name not in by_name:
            continue
        for addr in item.get("addr_info", []):
            family = addr.get("family")
            if family not in ("inet", "inet6"):
                continue
            prefix = int(addr["prefixlen"])
            by_name[name]["addresses"].append({
                "family": 4 if family == "inet" else 6,
                "address": str(ipaddress.ip_address(addr["local"])),
                "prefixlen": prefix,
                "network": canonical_network(addr["local"], prefix),
            })
    return {"schema": SCHEMA, "interfaces": list(by_name.values()), "selected": interface}

def main() -> int:
    p = argparse.ArgumentParser()
    p.add_argument("--interface")
    p.add_argument("--json", action="store_true")
    p.add_argument("--require-interface")
    args = p.parse_args()
    try:
        result = collect(args.interface)
    except (RuntimeError, subprocess.CalledProcessError, json.JSONDecodeError, KeyError, ValueError) as e:
        print(f"network_get: {e}")
        return 3
    if args.require_interface and not any(i["name"] == args.require_interface for i in result["interfaces"]):
        print(f"network_get: interface not found: {args.require_interface}")
        return 4
    print(json.dumps(result, sort_keys=True) if args.json else json.dumps(result, indent=2, sort_keys=True))
    return 0

if __name__ == "__main__":
    raise SystemExit(main())
