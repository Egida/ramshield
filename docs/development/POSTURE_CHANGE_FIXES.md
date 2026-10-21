# POSTURE CHANGE — the control plane requires a verified principal for mutations

**Decision:** owner Decision 1, option **B** (see
`docs/development/OWNER_DECISION_PACKAGE.md`).
**Branch:** `fix/master-p0-p1-fixes` — base `e5caac1` (`origin/master`).
**Scope:** the IPC and dashboard state-changing paths. No other subsystem is
touched. F-02 and F-03 remain parked.

---

## 1. The defect

The RamShield control plane was unauthenticated by default, and the exposure
was not limited to reads.

With the default configuration (`ipc.auth_keys` empty, `ipc.require_auth`
false), `auth_enforced` is false, so `parse_frame` accepted a frame carrying
no `auth` envelope and returned `principal = None`. `process_request` then
**skipped the role table entirely** — the `if let Some(p) = principal` arm
simply did not run — and the enforcement command's `actor` fell back to a
hard-coded `"admin"`.

The consequence is a *write*, not a read. A process that can reach the IPC
socket could issue `unblock_ip` and remove an active mitigation from a
customer under attack. The behavioural proof, taken before the fix, is the
server's own answer:

```
{"type":"ok","message":"unblock queued for 203.0.113.7"}
```

The bind defaults to `127.0.0.1:7890`, so the surface was loopback-only, and
`Config::validate()` refuses a public bind unless HMAC keys are set and a
TLS-terminating proxy is asserted — so the exposure could not be widened past
loopback by configuration alone. But loopback is not a boundary between
mutually untrusted local processes, and "any local process can cancel a
mitigation" is a materially different posture from "any local process can
read a status page".

The dashboard had the same shape, one route narrower: with no
`dashboard.admin_password_hash` the auth middleware passed every request
through, including `POST /api/config`.

---

## 2. The change

### 2.1 The mutating set is defined once, in the protocol crate

`crates/ramshield-protocol/src/message.rs` gains `Request::is_mutation()` —
true for exactly `BlockIp`, `BlockCidr`, `UnblockIp`, `UnblockCidr` and
`Flush`. The IPC role table is *derived* from this list, not the other way
round, so the wire contract and the policy cannot drift apart.

### 2.2 The IPC refuses an unauthenticated mutation

`src/ipc/server/handlers.rs` — `process_request` refuses a mutating request
that carries no verified principal, with `401` and the existing
`inc_rejected` / `inc_ipc_auth_rejections` counters.

Reads and telemetry stay open on purpose: `report_connection` /
`report_connections` is the hot path from the proxy and carries hostile
inbound data, so authenticating it buys nothing. The telemetry path does feed
the detection pipeline, and detection can react to it — that residual is
recorded in `docs/IPC.md` rather than hidden. What this change closes is the
*removal* of a mitigation, not the *influence* over it.

### 2.3 The dashboard requires a session for the config patch

`src/dashboard/mod.rs` — `api_set_config` requires a valid session when
`dashboard.require_auth_for_config` is set. The check runs **after** the CSRF
check, so a cross-origin request is still reported `403` rather than `401`:
the origin is the more specific fact, and the status must not depend on
whether the caller happens to hold a session.

`src/dashboard/auth.rs` gains `session_valid()`, and the `require_auth`
middleware was refactored to call it, so there is exactly one cookie-parsing
implementation and the two cannot drift.

### 2.4 Both switches default to the secure value, on every path

`ipc.require_auth_for_mutations` and `dashboard.require_auth_for_config` both
use `#[serde(default = "default_true")]`. A bare `#[serde(default)]` on a
`bool` yields **false**, which here would be the insecure value — hence the
explicit helper (`crates/ramshield-config/src/lib.rs`).

Every path that could produce a false default was checked:

| Path | Result |
|---|---|
| Serde, key absent in TOML/JSON | `default_true` → `true` |
| Serde, whole `[ipc]` / `[dashboard]` section absent | section `Default` impl → `true` |
| `Config::default()` | section `Default` impls → `true` |
| `Config::load` / `from_toml_file` | plain `toml::from_str` + `validate`; no post-load rewrite → `true` |
| Env-var overrides | no env var exists for either flag; the env layer only assigns listed fields → cannot be flipped |
| `ConfigPatch` carrying a whole section | the section struct is deserialised by the same serde impls, so an *absent* key in an older config deserialises to `default_true` = `true`. A patch can set `false` only by saying so explicitly |

The dashboard gate reads the **live** config, not the incoming patch, so a
patch cannot disable the gate for the request that carries it — only for
later requests, and reaching that code already required a valid session.

### 2.5 The escape hatches cannot widen the exposure past loopback

`ipc.require_auth_for_mutations = false` and
`dashboard.require_auth_for_config = false` re-open the loopback path they
already govern. They cannot reach a public bind: `validate()` refuses a
public `ipc.tcp_addr` without HMAC keys (and with keys present, every frame
carries a principal and the role table applies), and refuses a public
`dashboard.http_addr` without an admin password hash (and with a hash
present, the `require_auth` middleware already gates every `/api/*` route).

Both switches are explicit-affirmative — `= false` must be typed — and
documented in `docs/CONFIGURATION.md`.

### 2.6 A weakening patch is not silent

An authenticated dashboard admin can relax either switch by patching it.
That is inside an admin's authority, but it is no longer silent: the patch
logs a warning naming the reopened surface. The patch is still validated, and
a patch that would widen an exposure — a public bind without credentials — is
rejected by `Config::validate()`.

---

## 3. The evidence

Every cycle below was observed with the **final** code, not an earlier draft.

### 3.1 The IPC gate — RED then GREEN

The behavioural RED, taken with the gate removed from the final tree:

```
an unauthenticated mutation must not be accepted:
  {"type":"ok","message":"unblock queued for 203.0.113.7"}
```

With the gate in place, the same request is refused `401` while the telemetry
request on the same socket still succeeds.

### 3.2 The dashboard gate — RED then GREEN

```
the config patch must not be open to any local process
```

with the gate removed, and 3/3 green with it in place.

### 3.3 The classification consistency test is discriminating

`mutating_classification_agrees_with_the_role_table` asserts, for **every**
request variant, that `is_mutation()` equals "a `ReadOnly` principal is denied
it but an `Admin` is allowed it". Removing `Flush` from the mutating set
makes it fail:

```
flush: is_mutation() disagrees with the role table
```

So a future variant added to the enum and to one table but not the other
fails the suite.

### 3.4 The route-authorization matrix

| Mode | Session | `POST /api/config` |
|---|---|---|
| no password hash | none | `401` — the route gate (`config_patch_requires_a_session_without_a_configured_password`) |
| no password hash, gate disabled | none | allowed — the escape hatch (`config_patch_is_allowed_when_the_gate_is_disabled`) |
| password hash configured | none | `401` — refused by the **auth middleware**, proven by the response body being `{"error":"unauthorized"}` and not a `ConfigResponse` (`config_patch_is_refused_without_a_session_when_auth_is_enabled`) |
| password hash configured | valid | `200`, the patch is applied (`post_config_same_origin_allowed`) |

The third row's test is discriminating: with the middleware layer removed the
response body becomes the route gate's `{"ok":false,"config":{…}}` and the
assertion fails.

### 3.5 The gates

All 14 CI-equivalent gates pass on the final tree — see §5.

---

## 4. What was deliberately NOT changed

- **The read and telemetry paths stay open.** They are the proxy's hot path
  and carry hostile inbound data; authenticating them buys nothing and would
  break the ingest path.
- **The dashboard's read routes stay open.** The change is limited to the one
  state-changing route.
- **The IPC role table is untouched.** It already denied `Operator` actions
  to weak keys; this change only covers the "no principal at all" case.
- **The mesh CRDT path is untouched.** Peer-to-peer replication is not the
  local operator control plane and is out of scope for this defect.
- **F-02 and F-03 remain parked** until explicitly authorised.

---

## 5. Verification

```
cargo fmt --all --check                          clean
cargo test --lib dashboard                       32 passed, 0 failed
cargo test --lib ipc                             17 passed, 0 failed
cargo test --test ipc_wiring                       6 passed, 0 failed
14-gate CI-equivalent suite                      14/14 EXIT=0
                                                   aggregate 1828 passed, 0 failed, 42 ignored
```

Gate log: `/tmp/f01/final_gates.log` (local scratch, not committed).

---

## 6. The one behaviour change an operator must know

**In the default no-credentials configuration, `POST /api/config` is now
always `401`.** A session cannot be minted without an admin password, so the
config patch is not reachable until either

- `dashboard.admin_password_hash` is set to a valid PHC string (restart, log
  in at `/login`, use the session cookie), or
- `dashboard.require_auth_for_config = false` is set in `config.toml`
  (restart).

Both are config-file changes and take effect at startup; there is no
in-process way to mint a session without a password. This is fail-closed and
correct, but it is a change for a single-user development setup that used the
dashboard to patch configuration. `docs/OPERATIONS.md` states the procedure.

---

## 7. Review

`docs/development/POSTURE_CHANGE_REVIEW.md` — independent review of this
change, **VERDICT: PASS / SAFE TO PUSH: yes**. Its eight non-blocking
observations were addressed in this batch:

| Observation | Response |
|---|---|
| 2 — the no-password config patch is now permanently 401; is the recovery documented? | §6 above and `docs/OPERATIONS.md` now state the two ways to restore it |
| 3 — telemetry can still feed detection, and therefore auto-blocks | Recorded explicitly in `docs/IPC.md` as a residual |
| 4 — an admin can weaken the IPC gate by patch | The weakening patch now logs a warning naming the reopened surface (§2.6) |
| 5 — `inc_csrf_allowed` is incremented before the session gate | Kept as is, with a comment recording that the counter measures the CSRF check alone |
| 6 — the role-table equivalence is a proxy, not the definition | `is_mutation()`'s doc comment now states the dependency runs one way: the role table is derived from it |
| 7 — `default_true` is `pub(crate)` | No action needed; noted here for the record |
| 8 — the doc bodies were not independently re-read | Re-read and updated in this batch (§2.4, §2.5) |
| 1 — RED→GREEN and the gate results were taken as given | Every cycle in §3 was re-observed with the final code |
