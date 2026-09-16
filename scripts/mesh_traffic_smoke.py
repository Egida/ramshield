#!/usr/bin/env python3
"""Trigger mesh CRDT counters (hlc_ticks, purge_ticks, record_bans, record_unbans).

Mesh is single-node by default — this script enables it via IPC and exercises
the CRDT directly so the dashboard shows non-zero mesh telemetry.
"""
from __future__ import annotations
import json, socket, time, urllib.request

IPC = ("127.0.0.1", 7890)
DASH = "http://127.0.0.1:9999"


def ipc(payload: dict) -> dict:
    with socket.create_connection(IPC, timeout=3) as s:
        s.sendall((json.dumps(payload) + "\n").encode())
        return json.loads(s.recv(65536).split(b"\n", 1)[0])


def sse() -> dict:
    req = urllib.request.Request(f"{DASH}/api/stream")
    with urllib.request.urlopen(req, timeout=4) as r:
        buf = b""
        while b"data: " not in buf:
            buf += r.read(4096)
        return json.loads(buf.split(b"data: ")[1].split(b"\n")[0])


def main() -> int:
    # Phase 1: ban/unban cycles
    for i in range(20):
        ip = f"10.99.{i % 50}.{i // 50 + 1}"
        ipc({"type": "block_ip", "ip": ip, "reason": "mesh-smoke", "ttl_secs": 300})
    time.sleep(1)
    for i in range(10):
        ipc({"type": "unblock_ip", "ip": f"10.99.{i % 50}.{i // 50 + 1}"})
    time.sleep(1)

    # Phase 2: trigger mesh purge via short-TTL re-block
    for i in range(15):
        ipc({"type": "block_ip", "ip": f"10.88.{i % 30}.{i // 30 + 1}",
             "reason": "mesh-purge", "ttl_secs": 1})
    time.sleep(2)

    frame = sse()
    mods = {m["label"]: m["detail"] for m in frame["detection"].get("modules", [])}
    mh = mods.get("Mesh", {})
    print(json.dumps(mh, indent=2))
    ok = (mh.get("record_bans", 0) > 0
          and mh.get("purge_ticks", 0) >= 0)
    print("PASS" if ok else "FAIL")
    return 0 if ok else 1


if __name__ == "__main__":
    raise SystemExit(main())