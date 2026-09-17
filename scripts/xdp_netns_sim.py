#!/usr/bin/env python3
"""xdp_netns_sim.py — simulate VMs with virtual IPs via network namespaces.

Topology:
  vm_netns → sv{N} → br_xdp_sim → xdp_test1 → (veth pair) → xdp_test0 RX → XDP

VMs are on the SAME L2 as xdp_test0 (192.0.2.0/24). ARP resolves
xdp_test0's MAC (c6:97:93:a2:7b:2d) across the bridge → veth pair.
XDP program on xdp_test0's RX processes every packet.

Run: sudo python3 scripts/xdp_netns_sim.py [--vms N] [--duration SEC]
"""
from __future__ import annotations
import argparse, json, socket, struct, subprocess, time, urllib.request

IPC = ("127.0.0.1", 7890)
DASH = "http://127.0.0.1:9999"

SUBNET = "192.0.2"
BASE_IP = 100          # VMs get 192.0.2.100+, .1 is xdp_test0
BRIDGE = "br_xdp_sim"
IP = "/usr/sbin/ip"
XDP_MAC = bytes([0xc6, 0x97, 0x93, 0xa2, 0x7b, 0x2d])  # xdp_test0 MAC


def run(cmd: list[str]) -> subprocess.CompletedProcess:
    return subprocess.run(cmd, capture_output=True, text=True, timeout=10)


def setup_netns(vm_id: int) -> str:
    """Create namespace + veth pair, host side enslaved to bridge."""
    h_veth, n_veth = f"sveth{vm_id}", f"sv{vm_id}"
    ip = f"{SUBNET}.{BASE_IP + vm_id}"
    ns = f"vm{vm_id}"
    run([IP, "netns", "add", ns])
    run([IP, "link", "add", h_veth, "type", "veth", "peer", "name", n_veth])
    run([IP, "link", "set", n_veth, "netns", ns])
    # Host side: no IP needed (L2 only, enslaved to bridge)
    run([IP, "link", "set", h_veth, "up"])
    run([IP, "link", "set", h_veth, "master", BRIDGE])
    # Guest side
    run([IP, "netns", "exec", ns, IP, "link", "set", "lo", "up"])
    run([IP, "netns", "exec", ns, IP, "addr", "add", f"{ip}/24", "dev", n_veth])
    run([IP, "netns", "exec", ns, IP, "link", "set", n_veth, "up"])
    print(f"  vm{vm_id}: {ip} (L2 on bridge)")
    return ip


def teardown_netns(vm_id: int) -> None:
    run([IP, "link", "del", f"sveth{vm_id}"])
    run([IP, "netns", "del", f"vm{vm_id}"])


def flood_vm(vm_id: int, duration: float, pps_target: int = 10000) -> dict:
    """Flood UDP packets from netns to xdp_test0. Returns {sent, elapsed}."""
    ns = f"vm{vm_id}"
    # Python inside netns: send raw UDP packets via AF_INET
    # Packets go through the namespace's veth → bridge → xdp_test1 → xdp_test0
    flood_script = f'''
import socket, time, os
sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
sock.setblocking(False)
dest = ("192.0.2.1", 9999)
sent = 0
end = time.monotonic() + {duration}
while time.monotonic() < end:
    try:
        sock.sendto(b"XDP_SIM_" * 16, dest)
        sent += 1
    except BlockingIOError:
        pass
print(f"SENT={{sent}}")
os._exit(0)
'''
    result = subprocess.run(
        [IP, "netns", "exec", ns, "python3", "-c", flood_script],
        capture_output=True, text=True, timeout=int(duration) + 15
    )
    sent = 0
    for line in (result.stdout + result.stderr).splitlines():
        if line.startswith("SENT="):
            sent = int(line.split("=")[1])
    return {"sent": sent, "elapsed": duration}


def block_cidr(cidr: str) -> dict:
    with socket.create_connection(IPC, timeout=3) as sock:
        body = json.dumps({
            "type": "block_cidr", "cidr": cidr,
            "reason": "xdp-cidr-verify", "ttl_secs": 30,
        }) + "\n"
        sock.sendall(body.encode())
        return json.loads(sock.recv(65536).split(b"\n", 1)[0])


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


def main() -> int:
    parser = argparse.ArgumentParser(description="Simulate VMs hitting XDP via netns+bridge")
    parser.add_argument("--vms", type=int, default=5)
    parser.add_argument("--duration", type=float, default=5.0, help="flood seconds per VM")
    parser.add_argument("--keep", action="store_true")
    parser.add_argument("--cidr", help="block this CIDR through IPC before flooding")
    parser.add_argument("--ipc", default="127.0.0.1:7890")
    parser.add_argument("--dashboard", default="http://127.0.0.1:9999")
    args = parser.parse_args()
    global IPC, DASH
    host, port = args.ipc.rsplit(":", 1)
    IPC = (host, int(port))
    DASH = args.dashboard

    print(f"=== xdp_netns_sim: {args.vms} VMs, {args.duration}s each ===")
    print(f"    Subnet: {SUBNET}.0/24  Bridge: {BRIDGE}  XDP target: 192.0.2.1")

    # Verify XDP active
    snap_req = urllib.request.Request("http://127.0.0.1:9999/api/snapshot")
    with urllib.request.urlopen(snap_req, timeout=3) as r:
        snap = json.loads(r.read())
    if not snap.get("xdp_active"):
        print("FAIL: xdp_active=false")
        return 1

    # Create bridge (L2 only, no IP — VMs and xdp_test0 share 192.0.2.0/24)
    run([IP, "link", "add", BRIDGE, "type", "bridge"])
    run([IP, "link", "set", BRIDGE, "up"])
    # Enslave xdp_test1 to bridge — this is the path to xdp_test0's RX
    run([IP, "link", "set", "xdp_test1", "nomaster"])
    run([IP, "link", "set", "xdp_test1", "master", BRIDGE])
    print(f"Bridge {BRIDGE}: xdp_test1 enslaved")

    # Create VMs (all on 192.0.2.{100+}, same L2 as xdp_test0)
    for i in range(args.vms):
        setup_netns(i)
    time.sleep(2)  # let ARP resolve

    # Baseline
    before = xdp_counters()
    print(f"\nBaseline: wire_pass={before['wire_pass_total']} v4_drops={before['v4_drops_total']}")
    if args.cidr:
        response = block_cidr(args.cidr)
        if response.get("type") != "ok":
            raise RuntimeError(f"CIDR block rejected: {response}")
        time.sleep(1)  # enforcement actor applies the queued LPM update
        print(f"CIDR installed: {args.cidr}")

    # Sequential flood (parallel subprocesses on same netns conflict)
    total_sent = 0
    for i in range(args.vms):
        result = flood_vm(i, args.duration)
        print(f"  vm{i} ({SUBNET}.{BASE_IP + i}): {result['sent']} packets")
        total_sent += result["sent"]

    time.sleep(2)  # let counters propagate to SSE
    after = xdp_counters()

    pass_delta = after['wire_pass_total'] - before['wire_pass_total']
    drop_delta = after['v4_drops_total'] - before['v4_drops_total']

    print(f"\nResults:")
    print(f"  Packets sent: {total_sent:,}")
    print(f"  wire_pass: {before['wire_pass_total']} → {after['wire_pass_total']} (+{pass_delta})")
    print(f"  v4_drops:  {before['v4_drops_total']} → {after['v4_drops_total']} (+{drop_delta})")
    print(f"  Throughput: {total_sent / (args.vms * args.duration):.0f} pps")

    if args.cidr:
        ok = total_sent > 0 and drop_delta > 0
        print("\nPASS: CIDR traffic dropped by XDP" if ok else "\nFAIL: CIDR drop counter unchanged")
    else:
        ok = pass_delta > 0 or drop_delta > 0
        print("\nPASS: VM traffic flowing through XDP eBPF" if ok else "\nFAIL: no counter change")
    if not ok:
        return 1

    # Teardown
    if not args.keep:
        for i in range(args.vms):
            teardown_netns(i)
        run([IP, "link", "set", "xdp_test1", "nomaster"])
        run([IP, "link", "del", BRIDGE])
        print("Teardown complete")
    else:
        print(f"Kept. Cleanup: for i in $(seq 0 {args.vms-1}); do ip netns del vm$i; done; ip link del {BRIDGE}; ip link set xdp_test1 nomaster")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
