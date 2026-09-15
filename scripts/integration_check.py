#!/usr/bin/env python3
"""
Full integration test verification for RamShield.
Validates all module relationships against audit findings.
"""
import subprocess, sys, json, time, signal, os, socket, urllib.request

REPO = __file__[:-len("scripts/integration_check.py")]
BIN = REPO + "/target/release/ramshield"
IPC_HOST, IPC_PORT = "127.0.0.1", 17890
DASH_PORT = 19999
DASH_URL = f"http://127.0.0.1:{DASH_PORT}"

def ipc(payload: dict, key_hex: str = "", timeout=5.0) -> dict:
    frame = dict(payload)
    if key_hex:
        import hmac as h, hashlib
        ts = int(time.time() * 1000)
        kid = "k1"
        payload_str = json.dumps(frame, separators=(",", ":"), sort_keys=True).encode()
        sig = h.new(bytes.fromhex(key_hex), f"{ts}.{kid}".encode() + payload_str, hashlib.sha256).hexdigest()
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
        s.proc = subprocess.Popen([BIN, "config.toml"], cwd=REPO,
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

def detection():
    c = C("detection")
    with Server():
        # Get full config (needed for proper merge)
        st, body = dash("/api/config")
        v = json.loads(body)
        # Set low threshold using full detection config
        det = dict(v.get("detection", {}))
        det["rps_threshold"] = 5
        st, body = dash("/api/config", method="POST", body={"detection": det})
        if st != 200:
            c.ok(False, "config update", body[:200])
            return c.done()
        # Each flush = 200 events = 200 rps, threshold=5.
        # EWMA debounce needs 2 consecutive hot samples;
        # prev_sample_hot persists in IpRecord between flushes.
        ev = [{"ip":"198.51.100.66","bytes":512,"status_code":200,"proto_fp":4096} for _ in range(200)]
        blocked = False
        for _ in range(10):
            ipc({"type":"report_connections","events":ev})
            time.sleep(0.2)
            if ipc({"type":"check_ip","ip":"198.51.100.66"}).get("blocked"):
                blocked = True; break
        c.ok(blocked, "EWMA auto-block fires above threshold")
        # Subnet /24 block: >50 unique IPs per /24 per window.
        evs = [{"ip":f"192.0.2.{i%254}","bytes":256,"status_code":404,"proto_fp":4096} for i in range(200)]
        for _ in range(5):
            ipc({"type":"report_connections","events":evs})
            time.sleep(0.2)
        sub = False
        for _ in range(20):
            time.sleep(0.1)
            if ipc({"type":"check_ip","ip":"192.0.2.100"}).get("blocked"):
                sub = True; break
        c.ok(sub, "subnet /24 block on distinct-IP flood")
        c.ok(dash("/api/traffic/subnets")[0]==200, "hot-subnets reachable")
    return c.done()

def audit_static():
    c = C("audit_static")
    # Heavy cargo gate lives in audit_verify.sh; here keep it <5s so the
    # combined suite (audit_static + 5 live-server modules) finishes before
    # the terminal timeout. Run `bash scripts/audit_verify.sh` for the full gate.
    r = subprocess.run(["bash","scripts/audit_verify.sh","--quick"], cwd=REPO, capture_output=True, text=True, timeout=10)
    tail = (r.stdout + r.stderr)[-800:] if r.returncode else ""
    c.ok(r.returncode == 0, "audit_verify --quick", tail)
    r = subprocess.run(["rg","-n","--type","rust","--glob","!tests/**","--glob","!**/tests/**","--glob","!**/*.test.rs","--glob","!**/test_*.rs","--glob","!**/*_test.rs","-e",r"\.unwrap\s*\(", "-e",r"\.expect\s*\(", "src/","crates/"], cwd=REPO, capture_output=True, text=True)
    f = subprocess.run(["python3",".github/scripts/ci_filter_testcode.py","."], input=r.stdout, cwd=REPO, capture_output=True, text=True)
    legit = ("spawn batch processor","spawn subnet batch loop","OUT_DIR","try_into().unwrap()","ponytail:")
    kept = [l for l in f.stdout.splitlines() if l.strip() and not any(x in l for x in legit)]
    c.ok(not kept, "no-unwrap gate (legit sites excluded)", "\n".join(kept[:6]))
    src = open(REPO + "/src/ipc/server.rs").read()
    c.ok("config: ConfigHandle" in src, "H3 IpcServer holds ConfigHandle")
    c.ok("live_keys" in src, "H3 per-connection live keys")
    c.ok("config: &Config" not in src, "H3 old &Config sig gone")
    eng = open(REPO + "/src/engine/mod.rs").read()
    c.ok("cfg_handle.clone()" in eng, "H3 engine passes handle")
    auth = open(REPO + "/src/dashboard/auth.rs").read()
    c.ok("Path=/" in auth and "DashMap" in auth, "dashboard Path=/ + DashMap")
    cfg = open(REPO + "/crates/ramshield-config/src/lib.rs").read()
    c.ok("is_ascii_hexdigit" in cfg, "validate hex keys")
    c.ok("PasswordHash::new(" in cfg, "validate PHC hash")
    proto = open(REPO + "/crates/ramshield-protocol/src/auth.rs").read()
    c.ok("key_id" in proto, "sign binds key_id")
    return c.done()

print("=== RamShield Full Integration Verification ===")
print("Testing all module relationships per AUDIT_FULL_20260911.md\n")

# Run all checks
results = []
results.append(("audit_static", audit_static()))
results.append(("ipc_basic", subprocess.run([sys.executable, __file__, "ipc_basic"], cwd=REPO).returncode))
results.append(("ipc_auth", subprocess.run([sys.executable, __file__, "ipc_auth"], cwd=REPO).returncode))
results.append(("dash_api", subprocess.run([sys.executable, __file__, "dash_api"], cwd=REPO).returncode))
results.append(("config_api", subprocess.run([sys.executable, __file__, "config_api"], cwd=REPO).returncode))
results.append(("cli", subprocess.run([sys.executable, __file__, "cli"], cwd=REPO).returncode))
results.append(("detection", detection()))

print("=== Summary ===")
for name, status in results:
    state = "PASS" if status == 0 else "FAIL"
    print(f"  [{state}] {name}")

sys.exit(0 if all(s == 0 for _, s in results) else 1)
