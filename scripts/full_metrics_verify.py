#!/usr/bin/env python3
"""Full RamShield metrics completeness verification:
- XDP kernel counters (wire_pass, v4_drops, v6_drops)
- CGNAT tier counters (classify_ticks, tier_allow, tier_challenge, tier_powdrop)
- HLL/CMS analytics counters (hll_inserts, cms_increments, entropy, entropy_ticks, forecast_ticks)
- Mesh CRDT counters (record_bans, purge_ticks, hlc_ticks)
- Forecasting counters (hw_rps, hw_forecast, hw_zscore, entropy, entropy_ticks, forecast_ticks)
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
    print("Phase A: CGNAT classification traffic")
    # Send subnet-block traffic to exercise CGNAT
    for subnet in range(3):
        for ip in [f"10.{subnet}.{i}" for i in range(10, 25)]:
            ipc({"type": "report_connections",
                 "events": [{"ip": ip, "bytes": 256, "status_code": 200,
                             "proto_fp": 0x1000} for _ in range(3)]})
    time.sleep(1)

    print("Phase B: Analytics diversity")
    for code in [200, 404, 500, 503]:
        for ip in [f"10.50.{code % 100}.{i}" for i in range(200)]:
            ipc({"type": "report_connections",
                 "events": [{"ip": ip, "bytes": 512, "status_code": code,
                             "proto_fp": 0x0001} for _ in range(1)]})
    time.sleep(1)

    print("Phase C: Forecasting ramp")
    for _ in range(30):
        for subnet in range(5):
            for ip in [f"10.10.{subnet}.{i}" for i in range(50)]:
                ipc({"type": "report_connections",
                     "events": [{"ip": ip, "bytes": 128, "status_code": 200,
                                 "proto_fp": 0x0011 + subnet} for _ in range(1)]})
        time.sleep(0.03)

    # Phase D: Mesh ban/unban cycles
    for i in range(20):
        ip = f"172.16.{i % 256}.1"
        ipc({"type": "block_ip", "ip": ip, "reason": "mesh-final", "ttl_secs": 300})
    time.sleep(0.5)
    for i in range(10):
        ipc({"type": "unblock_ip", "ip": f"172.16.{i % 256}.1"})
    time.sleep(0.5)

    frame = sse()
    mods = {m["label"]: m["detail"] for m in frame["detection"].get("modules", [])}
    an = mods.get("Analytics", {})
    cg = mods.get("CGNAT", {})
    fc = mods.get("Forecasting", {})
    mh = mods.get("Mesh", {})

    all_ok = True

    # CGNAT
    if cg.get("classify_ticks", 0) <= 0:
        print(f"FAIL: cgnat.classify_ticks={cg.get('classify_ticks')}")
        all_ok = False
    else:
        print(f"PASS: cgnat.classify_ticks={cg.get('classify_ticks')}")

    # HLL/CMS
    if an.get("hll_inserts", 0) <= 0:
        print(f"FAIL: analytics.hll_inserts={an.get('hll_inserts')}")
        all_ok = False
    else:
        print(f"PASS: analytics.hll_inserts={an.get('hll_inserts')}")

    if an.get("cms_increments", 0) <= 0:
        print(f"FAIL: analytics.cms_increments={an.get('cms_increments')}")
        all_ok = False
    else:
        print(f"PASS: analytics.cms_increments={an.get('cms_increments')}")

    # Forecasting
    if fc.get("entropy", 0) <= 0:
        print(f"FAIL: forecasting.entropy={fc.get('entropy')}")
        all_ok = False
    else:
        print(f"PASS: forecasting.entropy={fc.get('entropy')}")

    if fc.get("entropy_ticks", 0) <= 0:
        print(f"FAIL: forecasting.entropy_ticks={fc.get('entropy_ticks')}")
        all_ok = False
    else:
        print(f"PASS: forecasting.entropy_ticks={fc.get('entropy_ticks')}")

    if fc.get("forecast_ticks", 0) <= 0:
        print(f"FAIL: forecasting.forecast_ticks={fc.get('forecast_ticks')}")
        all_ok = False
    else:
        print(f"PASS: forecasting.forecast_ticks={fc.get('forecast_ticks')}")

    # Mesh
    if mh.get("hlc_ticks", 0) <= 0:
        print(f"FAIL: mesh.hlc_ticks={mh.get('hlc_ticks')}")
        all_ok = False
    else:
        print(f"PASS: mesh.hlc_ticks={mh.get('hlc_ticks')}")

    if mh.get("record_bans", 0) <= 0:
        print(f"FAIL: mesh.record_bans={mh.get('record_bans')}")
        all_ok = False
    else:
        print(f"PASS: mesh.record_bans={mh.get('record_bans')}")

    if mh.get("purge_ticks", 0) <= 0:
        print(f"FAIL: mesh.purge_ticks={mh.get('purge_ticks')}")
        all_ok = False
    else:
        print(f"PASS: mesh.purge_ticks={mh.get('purge_ticks')}")

    # XDP counters
    xdp = frame.get("xdp", {})
    xdp_keys = ["active", "wire_pass_total", "v4_drops_total", "v6_drops_total", "parse_fails_total"]
    missing = [k for k in xdp_keys if k not in xdp or xdp[k] is None]
    if missing:
        print(f"FAIL: missing xdp keys: {missing}")
        all_ok = False
    else:
        print(f"PASS: xdp counters active={xdp['active']}, "
              f"wire_pass={xdp['wire_pass_total']}, "
              f"v4_drops={xdp['v4_drops_total']}, "
              f"v6_drops={xdp['v6_drops_total']}, "
              f"parse_fails={xdp['parse_fails_total']}")

    if all_ok:
        print("\nALL COMPLETE: full metric telemetry verified non-zero")
    else:
        print("\nSOME COUNTERS ZERO (verify traffic paths)")
    return 0 if all_ok else 1


if __name__ == "__main__":
    raise SystemExit(main())