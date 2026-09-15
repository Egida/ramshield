#!/usr/bin/env python3
"""integration_lib.py — shared helpers for RamShield integration tests.
Boots release binary on scratch ports, drives IPC + dashboard.
Run:  python3 scripts/integration_check.py <name>
Names: audit_static, ipc_basic, ipc_auth, dash_api, detection, config_api, cli
"""
from __future__ import annotations
import json, os, signal, socket, subprocess, sys, time, urllib.request
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
BIN = REPO / "target" / "release" / "ramshield"
IPC_HOST, IPC_PORT = "127.0.0.1", 17890
DASH_PORT = 19999
DASH_URL = f"http://127.0.0.1:{DASH_PORT}"

def ipc(payload: dict, key_hex: str = "", timeout=5.0) -> dict:
    frame = dict(payload)
    if key_hex:
        import hmac as h, hashlib
        ts = int(time.time() * 1000)
        kid = "k1"
        # Server verifies over the frame WITHOUT the auth object — sign the
        # payload only, then embed auth separately. (CLI does the same.)
        payload_str = json.dumps(frame, separators=(",", ":"), sort_keys=True).encode()
        sig = h.new(bytes.fromhex(key_hex), f"{ts}.{kid}".encode() + payload_str,
                    hashlib.sha256).hexdigest()
        frame = {"auth": {"key_id": kid, "ts_ms": ts, "sig": sig}, **frame}
    with socket.create_connection((IPC_HOST, IPC_PORT), timeout=timeout) as s:
        s.sendall((json.dumps(frame) + "\n").encode())
        buf = b""
        while b"\n" not in buf:
            chunk = s.recv(65536)
            if not chunk: break
            buf += chunk
    return json.loads(buf.split(b"\n")[0])

def dash(path: str, method="GET", body=None, timeout=5):
    req = urllib.request.Request(DASH_URL + path, method=method,
        data=json.dumps(body).encode() if body is not None else None,
        headers={"Content-Type": "application/json"})
    try:
        with urllib.request.urlopen(req, timeout=timeout) as r:
            return r.status, r.read().decode()
    except urllib.error.HTTPError as e:
        return e.code, e.read().decode()

def wait_ready(deadline=30.0) -> bool:
    end = time.monotonic() + deadline
    while time.monotonic() < end:
        try:
            with urllib.request.urlopen(f"{DASH_URL}/healthz", timeout=1) as r:
                if r.status == 200: return True
        except Exception: time.sleep(0.25)
    return False

class Server:
    def __init__(s, extra_env=None):
        s.proc = None; s.extra = extra_env or {}
    def __enter__(s):
        env = dict(os.environ, RAMSHIELD_IPC__TCP_ADDR=f"{IPC_HOST}:{IPC_PORT}",
                   RAMSHIELD_DASHBOARD__HTTP_ADDR=f"127.0.0.1:{DASH_PORT}",
                   RAMSHIELD_DASHBOARD__ENABLED="true", **s.extra)
        s.proc = subprocess.Popen([str(BIN), "config.toml"], cwd=str(REPO),
            env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
            start_new_session=True)
        if not wait_ready():
            s.__exit__(None, None, None)
            raise SystemExit("server never healthy")
        return s
    def __exit__(s, *e):
        if s.proc:
            try: os.killpg(s.proc.pid, signal.SIGTERM)
            except ProcessLookupError: pass
            try: s.proc.wait(timeout=5)
            except subprocess.TimeoutExpired: os.killpg(s.proc.pid, signal.SIGKILL)

class C:
    def __init__(s, n): s.n, s.p, s.f = n, 0, []
    def ok(s, cond, d, det=""):
        print(f"  [{'PASS' if cond else 'FAIL'}] {d}" + (f" — {det}" if det and not cond else ""))
        if cond: s.p += 1
        else: s.f.append(d)
    def done(s):
        t = s.p + len(s.f)
        print(f"== {s.n}: {'OK' if not s.f else 'FAILED'} ({s.p}/{t})\n")
        return len(s.f)
