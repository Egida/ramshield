# Deploy / release notes

Pilot: single node, loopback or private bind, operator TLS proxy if public.

## Service user

Run as unprivileged user. XDP needs file capabilities on the binary, not root shell:

```bash
sudo setcap 'cap_net_admin,cap_perfmon,cap_bpf+eip' /usr/local/bin/ramshield
```

Rebuild drops file caps — re-apply after every `cargo build`.

## Paths

| What | Prod example | Smoke |
|---|---|---|
| Config | `/etc/ramshield/config.toml` | `config.prod.toml` |
| WAL | dedicated dir, owner = service user | `/tmp/ramshield_wal` |
| Binary | `/usr/local/bin/ramshield` | `./target/release/ramshield` |

WAL dir must be writable. GroupCommit durability. Single writer.

## Config (must)

- `[wal] enabled = true`
- `[ipc] auth_keys` non-empty; `key_roles` for each key
- `[ipc] tcp_addr` loopback **or** `behind_tls_proxy = true`
- `[dashboard] http_addr` loopback; set `admin_password_hash` in real prod
- `[xdp] enabled` + `interface` + `mode` explicit

Dev: `config.dev.toml` (XDP off, WAL off, open IPC). Prod-shape: `config.prod.toml`.

## Startup

```bash
./target/release/ramshield --config /etc/ramshield/config.toml
# CI / no caps:
./target/release/ramshield --config config.prod.toml --no-xdp
```

No `--help`. Flags: `--config`/`-c`, `--no-xdp`, `--version`. Unknown flags fatal.

## Health

`GET /healthz` (no auth):

| HTTP | `status` | `reason` |
|---|---|---|
| 200 | `ok` | `running` |
| 503 | `degraded` | `starting` / `pipeline failed` / `ram pressure` / `shutting down` |

Also `xdp_active` bool. Process-alive ≠ pipeline-ready (P0#1).

## Shutdown / restart

SIGINT / SIGTERM / SIGHUP → drain workers → RAII XDP detach. Restart replays WAL then reconciles XDP.

SIGKILL drill: `scripts/prod_smoke.sh` kills -9 after a block and asserts `check_ip` still blocked.

## Rollback

Keep previous binary + previous config + WAL dir. Stop new, restore binary, start with same WAL path. No schema bump in P1–P8.

## Release evidence (fill per cut)

| Field | Value |
|---|---|
| version | `0.2.0` (`CARGO_PKG_VERSION`) |
| git | `git rev-parse HEAD` |
| lockfile | `Cargo.lock` committed |
| SBOM / signature | **not produced** |
| smoke | `CFG=./config.prod.toml ./scripts/prod_smoke.sh` |

Unit exists: `deploy/systemd/ramshield.service`, k8s DaemonSet, Containerfile — **not** a signed-digest drill.
