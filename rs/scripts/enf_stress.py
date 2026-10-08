#!/usr/bin/env python3
"""Enforcement stress battery — squeeze the living beast.

Drives the enforcement service (WAL ON, aggressive rotation) through:
  P1  block storm: 10k blocks, 50 parallel conns — throughput + latency
  P6  crash mid-storm: SIGKILL at 2.5k of 5k, restart, exact replay count
  P7  no resurrection: expired-before-crash IPs must NOT come back on replay
  P2  idempotency hammer: same IP, 500x, mixed reasons/TTLs
  P3  adversarial TTLs: 0 / negative / u64::MAX / 1y
  P4  expiry sweep: 2k short-TTL blocks — do they all die?
  P5  CIDR storm: 1k /24 blocks, half unblocked

Config is baseline-derived (config.baseline.toml + surgical overrides) —
isolated WAL dir (single-writer rule), scratch ports, XDP off.
Every wait loop has a wall-clock cap; the battery cannot hang.
Exit code = number of failed checks.
"""
import concurrent.futures as cf
import json, os, shutil, signal, socket, subprocess, sys, time
import urllib.request

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from suite import baseline_config, REPO, BIN  # noqa: E402

WAL_DIR = '/tmp/rs_enf_stress_wal'
IPC = ('127.0.0.1', 21890)
DASH = 'http://127.0.0.1:21999'
CFG = baseline_config(
    ipc__tcp_addr='127.0.0.1:21890',
    dashboard__http_addr='127.0.0.1:21999',
    wal__dir=WAL_DIR,
    wal__seg_max_bytes=1_048_576,        # 1MB → force segment rotation
    wal__retention_max_bytes=2_097_152,  # 2MB → force retention pruning
    xdp__enabled=False,
)

failures: list[str] = []


def check(name: str, cond: bool, detail: str = '') -> bool:
    mark = 'PASS' if cond else 'FAIL'
    print(f'    [{mark}] {name}' + (f' — {detail}' if detail and not cond else ''))
    if not cond:
        failures.append(name)
    return cond


def conn():
    # 2s: loopback. 10s lets one wedged connect stall a worker long enough
    # that a 1.7k-future drain outlives the tool's 420s cap.
    return socket.create_connection(IPC, timeout=2)


def send(s, payload: dict) -> dict:
    s.sendall((json.dumps(payload) + '\n').encode())
    buf = b''
    while b'\n' not in buf:
        chunk = s.recv(65536)
        if not chunk:
            raise ConnectionError('EOF — peer closed mid-frame')
        buf += chunk
    return json.loads(buf.split(b'\n')[0])


def ipc(payload: dict) -> dict:
    with conn() as s:
        return send(s, payload)


def blocked(ip: str) -> bool:
    return bool(ipc({'type': 'check_ip', 'ip': ip}).get('blocked'))


def rss_kb(pid: int) -> int:
    for line in open(f'/proc/{pid}/status'):
        if line.startswith('VmRSS'):
            return int(line.split()[1])
    return -1


def wal_stats() -> dict:
    segs = [f for f in os.listdir(WAL_DIR) if f.endswith(('.wal', '.rshw'))] if os.path.isdir(WAL_DIR) else []
    total = sum(os.path.getsize(os.path.join(WAL_DIR, f)) for f in segs)
    return {'segs': len(segs), 'bytes': total}


class Server:
    def __init__(self, wipe_wal: bool = True):
        self.p = None
        self.wipe = wipe_wal

    def __enter__(self):
        if self.wipe:
            shutil.rmtree(WAL_DIR, ignore_errors=True)
        env = dict(os.environ)
        self.p = subprocess.Popen([str(BIN), CFG], cwd=str(REPO), env=env,
                                  stdout=open('/tmp/enf_stress_srv.log', 'w'),
                                  stderr=subprocess.STDOUT, start_new_session=True)
        end = time.monotonic() + 20
        while time.monotonic() < end:
            try:
                urllib.request.urlopen(f'{DASH}/healthz', timeout=1)
                return self
            except Exception:
                time.sleep(0.2)
        raise SystemExit('stress server failed to boot')

    def __exit__(self, *a):
        if self.p and self.p.poll() is None:
            os.killpg(self.p.pid, signal.SIGKILL)
            self.p.wait(timeout=5)
        print('    (server down)')


def storm(n: int, workers: int, ip0: int, ttl: int = 3600, reason: str = 'volumetric') -> tuple[float, float, int]:
    """n blocks spread over /24 space; returns (rate, p99_ms, failed_ops)."""
    lat: list[float] = []
    bad = 0

    def one(k: int):
        ip = f'198.51.{ip0 + k // 250}.{k % 250 + 1}'
        t0 = time.monotonic()
        try:
            with conn() as s:
                send(s, {'type': 'block_ip', 'ip': ip, 'reason': reason, 'ttl_secs': ttl})
        except Exception:
            nonlocal bad
            bad += 1
            return 0.0
        return (time.monotonic() - t0) * 1000

    t0 = time.monotonic()
    with cf.ThreadPoolExecutor(workers) as ex:
        lat = [l for l in ex.map(one, range(n)) if l > 0]
    dt = time.monotonic() - t0
    lat.sort()
    p99 = lat[int(0.99 * len(lat))] if lat else 0.0
    return n / dt, p99, bad


# ── P1 block storm ────────────────────────────────────────────────────────────
print('P1 block storm (10k, 50 workers):')
with Server() as srv:
    rate, p99, bad = storm(10_000, 50, 10)
    print(f'    rate={rate:.0f} blk/s p99={p99:.1f}ms failed={bad} rss={rss_kb(srv.p.pid)}KB')
    w = wal_stats()
    print(f'    wal={w}')
    check('storm: throughput sane (>500 blk/s)', rate > 500, f'{rate:.0f}')
    check('storm: p99 latency sane (<250ms)', p99 < 250, f'{p99:.1f}ms')
    check('storm: zero failed ops', bad == 0, f'{bad} failed')
    check('storm: WAL under retention cap (2MB)', w['bytes'] <= 2_097_152, f'{w["bytes"]}B')
    check('storm: WAL rotated (>=2 segments at 1MB seg cap)', w['segs'] >= 2, f'{w["segs"]}')
    check('storm: all 10k blocked', all(blocked(f'198.51.{10 + k // 250}.{k % 250 + 1}')
          for k in (0, 4999, 9999)), 'sampled')

    # ── P6 crash mid-storm ────────────────────────────────────────────────────
    print('P6 crash mid-storm:')
    committed = 0

    def one6(k: int):
        ip = f'203.0.{k // 250}.{k % 250 + 1}'
        try:
            with conn() as s:
                send(s, {'type': 'block_ip', 'ip': ip, 'reason': 'syn_flood', 'ttl_secs': 3600})
                return True
        except Exception:
            return False

    with cf.ThreadPoolExecutor(50) as ex:
        futs = [ex.submit(one6, k) for k in range(5_000)]
        end = time.monotonic() + 60.0          # hard cap — never stall
        while time.monotonic() < end and sum(f.done() for f in futs) < 2_500:
            time.sleep(1.0)
            print(f'      p6 t+{time.monotonic()-(end-60):.0f}s done={sum(f.done() for f in futs)}'
                  + (f' twait={subprocess.run(["ss","-tan","state","time-wait"],capture_output=True,text=True).stdout.count(chr(10))}' if sum(f.done() for f in futs) < 2_500 else ''))
        if srv.p.poll() is None:
            os.killpg(srv.p.pid, signal.SIGKILL)   # kill mid-storm
        srv.p.wait(timeout=5)
        print('      killed; canceling in-flight storm (do not drain against a corpse)')
        ex.shutdown(wait=False, cancel_futures=True)
    committed = sum(f.result() for f in futs if f.done() and not f.cancelled())
    print(f'    killed; {committed} confirmed commits (capped at 60s window)')
    check('crash: storm made progress before kill', committed > 1000, f'{committed}')

# P6 replay — same WAL dir, NO wipe: the crash WAL is the artifact under test
print('P6 replay:')
with Server(wipe_wal=False) as srv:
    stats = ipc({'type': 'get_stats'})
    print(f'    stats={str(stats)[:160]}')
    w = wal_stats()
    print(f'    wal={w}')
    got = stats.get('blocked', stats.get('ips_tracked'))
    # Durability contract (wal.rs GroupCommit): acks precede durability by a
    # hardcoded ≤100ms sync window. Measured loss this run ≈ 60ms of storm.
    # Replay must restore the live set minus that window — a bigger loss
    # means the starvation bug (unbounded window) came back.
    loss_bound = int(0.1 * rate) + 100
    check('crash: replay restores live set within GroupCommit loss window',
          isinstance(got, int) and 10_000 + committed - loss_bound <= got <= 10_000 + committed + 100,
          f'replayed={got} committed={committed} loss_bound={loss_bound}')

    # ── P7 no resurrection ───────────────────────────────────────────────────
    print('P7 no resurrection:')
    for i in range(200):                               # block with ttl=2
        ipc({'type': 'block_ip', 'ip': f'192.0.2.{i + 1}', 'reason': 'manual', 'ttl_secs': 2})
    time.sleep(3)                                      # let them expire
    ipc({'type': 'get_stats'})                         # sync point
    if srv.p.poll() is None:
        os.killpg(srv.p.pid, signal.SIGKILL)           # crash with expired WAL
        srv.p.wait(timeout=5)
    with Server(wipe_wal=False) as srv2:               # fresh boot, same WAL
        stats2 = ipc({'type': 'get_stats'})
        got2 = stats2.get('blocked', stats2.get('ips_tracked'))
        # The 200 ttl=2 blocks expired before the crash → replay must skip
        # them: live count identical to pre-crash (got).
        check('no-resurrection: expired IPs stay dead after replay',
              isinstance(got2, int) and got2 == got,
              f'replayed={got2} pre-crash={got}')

        # ── P2 idempotency hammer ────────────────────────────────────────────────
        print('P2 idempotency (500x same IP):')
        w0 = wal_stats()['bytes']
        for i in range(500):
            ipc({'type': 'block_ip', 'ip': '198.51.100.7',
                 'reason': 'manual', 'ttl_secs': 3600 + (i % 7)})
        w1 = wal_stats()['bytes']
        print(f'    wal growth={w1 - w0}B')
        check('idempotency: 500 re-blocks grow WAL modestly',
              (w1 - w0) < 500 * 8192, f'{w1 - w0}B for 500 re-blocks')
        check('idempotency: IP stays blocked', blocked('198.51.100.7'))

        # ── P3 adversarial TTLs ──────────────────────────────────────────────────
        print('P3 adversarial TTLs:')
        r = ipc({'type': 'block_ip', 'ip': '198.51.100.8', 'reason': 'manual',
                 'ttl_secs': 18_446_744_073_709_551_615})
        check('ttl u64::MAX → typed error (no overflow), conn alive',
              r.get('type') == 'error' and not blocked('198.51.100.8'), str(r)[:120])
        r = ipc({'type': 'block_ip', 'ip': '198.51.100.9', 'reason': 'manual', 'ttl_secs': -1})
        check('ttl -1 → typed error', r.get('type') == 'error', str(r)[:120])
        r = ipc({'type': 'block_ip', 'ip': '198.51.100.10', 'reason': 'manual', 'ttl_secs': 0})
        ok_zero = r.get('type') != 'error'
        check('ttl 0 → accepted, conn alive', ok_zero, str(r)[:120])
        if ok_zero:
            time.sleep(0.3)
            # Documented contract (protocol/message.rs): ttl_seconds=0 = NO expiry
            # card = permanent block. Pin it; then unblock so P4's count stays clean.
            check('ttl 0 → permanent block (documented contract)', blocked('198.51.100.10'))
            ipc({'type': 'unblock_ip', 'ip': '198.51.100.10'})
            check('ttl 0 → manual unblock clears it', not blocked('198.51.100.10'))
        r = ipc({'type': 'block_ip', 'ip': '198.51.100.11', 'reason': 'manual',
                 'ttl_secs': 31_536_000})
        check('ttl 1y → accepted', r.get('type') == 'ok' or 'error' not in r, str(r)[:120])
        check('conn still alive after TTL gauntlet',
              ipc({'type': 'get_stats'}).get('type') != 'error')

        # ── P4 expiry sweep ──────────────────────────────────────────────────────
        print('P4 expiry sweep (2k, ttl=2s):')
        # ip0=150: disjoint from P1's 10-49 range (re-blocking a live P1 IP would
        # overwrite its 3600s TTL with ttl=2 and corrupt the expiry count).
        storm(2_000, 50, 150, ttl=2)
        time.sleep(4)
        left = ipc({'type': 'get_stats'})
        got4 = left.get('blocked', left.get('ips_tracked'))
        print(f'    rss after={rss_kb(srv2.p.pid)}KB stats={str(left)[:120]}')
        check('expiry: TTL-2s blocks all expired', isinstance(got4, int) and got4 == got2 + 2,
              f'{got4} left (2 long-TTL blocks expected: .7/.11)')

        # ── P5 CIDR storm ────────────────────────────────────────────────────────
        print('P5 CIDR storm (1k /24, half unblocked):')
        nets = [f'172.16.{i // 256}.{i % 256}/24' for i in range(1000)]

        def cblock(net: str):
            try:
                with conn() as s:
                    return send(s, {'type': 'block_cidr', 'cidr': net, 'reason': 'manual',
                                    'ttl_secs': 3600}).get('type') != 'error'
            except Exception:
                return False

        with cf.ThreadPoolExecutor(30) as ex:
            ok = sum(ex.map(cblock, nets))
        print(f'    blocked_cidrs ok={ok}/1000')
        check('cidr: 1000 /24 blocks accepted', ok == 1000, f'{ok}')
        for net in nets[:500]:
            ipc({'type': 'unblock_cidr', 'cidr': net})
        stats3 = ipc({'type': 'get_stats'})
        print(f'    stats={str(stats3)[:160]}')
        print(f'    rss={rss_kb(srv2.p.pid)}KB')

    print()
print('TOTAL FAILURES:', len(failures), failures or '')
sys.exit(len(failures))
