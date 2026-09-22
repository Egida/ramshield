#!/usr/bin/env python3
"""Exercise kernel XDP counters with real Ethernet frames.

Requires: root/CAP_NET_RAW for AF_PACKET, xdp_test0/xdp_test1 veth pair,
RamShield listening on IPC 7890 and dashboard 9999 with XDP enabled.
"""
from __future__ import annotations
import json, socket, struct, time, urllib.request

IPC = ("127.0.0.1", 7890)
DASH = "http://127.0.0.1:9999"
TX_IFACE = "xdp_test1"
SRC_IP = "172.31.42.1"
DST_IP = "192.0.2.1"
DST_MAC = bytes.fromhex("c69793a27b2d")
SRC_MAC = bytes.fromhex("b21b183f5f3c")


def ipc(payload: dict) -> dict:
    with socket.create_connection(IPC, timeout=3) as s:
        s.sendall((json.dumps(payload) + "\n").encode())
        return json.loads(s.recv(65536).split(b"\n", 1)[0])


def frame(src_ip: str, dst_ip: str, seq: int) -> bytes:
    eth = DST_MAC + SRC_MAC + struct.pack("!H", 0x0800)
    payload = struct.pack("!I", seq) + b"ramshield-xdp"
    udp = struct.pack("!HHHH", 40000, 9999, 8 + len(payload), 0) + payload
    ip = struct.pack(
        "!BBHHHBBH4s4s", 0x45, 0, 20 + len(udp), seq & 0xffff,
        0, 64, 17, 0, socket.inet_aton(src_ip), socket.inet_aton(dst_ip)
    )
    return eth + ip + udp


def xdp_counters() -> dict:
    req = urllib.request.Request(f"{DASH}/api/stream")
    with urllib.request.urlopen(req, timeout=4) as r:
        buf = b""
        deadline = time.monotonic() + 4
        while time.monotonic() < deadline:
            buf += r.read(4096)
            for line in buf.splitlines():
                if line.startswith(b"data: "):
                    return json.loads(line[6:])["xdp"]
    raise RuntimeError("no SSE frame")


def send(count: int) -> None:
    s = socket.socket(socket.AF_PACKET, socket.SOCK_RAW, socket.htons(3))
    s.bind((TX_IFACE, 0))
    packet = frame(SRC_IP, DST_IP, 0)
    for i in range(count):
        s.send(packet[:14] + frame(SRC_IP, DST_IP, i)[14:])
    s.close()


def main() -> int:
    before = xdp_counters()
    if not before.get("active"):
        raise SystemExit("xdp_active=false; rebuild caps/config before running")
    send(200)
    time.sleep(1)
    passed = xdp_counters()
    ipc({"type": "block_ip", "ip": SRC_IP, "reason": "xdp-smoke", "ttl_secs": 30})
    time.sleep(1)
    send(500)
    time.sleep(1)
    after = xdp_counters()
    print(json.dumps({"before": before, "after_pass": passed, "after_block": after}, indent=2))
    pass_delta = passed["wire_pass_total"] - before["wire_pass_total"]
    drop_delta = (after["v4_drops_total"] - passed["v4_drops_total"])
    if pass_delta <= 0 or drop_delta <= 0:
        print(f"FAIL: pass_delta={pass_delta}, v4_drop_delta={drop_delta}")
        return 1
    print(f"PASS: wire_pass +{pass_delta}, v4_drops +{drop_delta}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
