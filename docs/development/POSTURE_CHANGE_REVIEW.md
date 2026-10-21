# POSTURE CHANGE REVIEW — IPC / Dashboard control-plane authentication

Scope: uncommitted working-tree changes on branch `fix/master-p0-p1-fixes`
in `/home/m/.cache/rs-master` (13 files, +453/-41). Reviewer read the full
`git diff` plus the surrounding call sites (protocol enum, IPC auth/role
table, `parse_frame`, handler dispatch, dashboard routes + middleware,
config `Default`/serde/env/validate/patch paths). No mutating commands were
run; the test results quoted in the task were taken as given.

VERDICT: PASS
SAFE TO PUSH: yes

---

## a) Is the mutating set correct and COMPLETE?

**YES — complete and correct.**

The protocol enum (`crates/ramshield-protocol/src/message.rs:52-85`) has exactly
11 variants: `CheckIp`, `BlockIp`, `BlockCidr`, `UnblockIp`, `UnblockCidr`,
`GetIpStats`, `GetStats`, `GetStatus`, `ReportConnection`, `ReportConnections`,
`Flush`. `Request::is_mutation()` (`message.rs:97-107`) covers all five
enforcement-mutating ones and nothing else.

Cross-checked against the *other* source of truth — the role table in
`src/ipc/server/auth.rs:58-73`: `BlockIp|BlockCidr|UnblockIp|UnblockCidr`
require `Operator`; `Flush` requires `Admin`; `CheckIp|GetIpStats|GetStats|
GetStatus` require `ReadOnly`; `ReportConnection(s)` require `Telemetry`.
The two sets coincide exactly, and the new test
`mutating_classification_agrees_with_the_role_table`
(`src/ipc/server/tests/mod.rs:242-298`) asserts the equivalence
mechanically for every variant, so a future variant added to one table and
not the other fails the test.

Alternative routes to enforcement state: `process_request` is the **only**
dispatcher of `Request` (grep: sole caller is `src/ipc/server/connection.rs:126`),
and the handler `match` in `src/ipc/server/handlers.rs:112-124` is exhaustive
over all 11 variants — no `_ =>` arm. The dashboard exposes no unblock/block
route (route table `src/dashboard/mod.rs:52-63` contains exactly one mutating
route, `POST /api/config`). The mesh path (`MeshMutationAction::Unblock`,
`src/engine/boot.rs:110`, `crates/ramshield-enforcement/src/service/mesh.rs:95`)
is peer-to-peer CRDT replication, not the local operator control plane, and is
out of scope for this defect. No read variant has a hidden enforcement side
effect: `handle_check_ip` / `handle_get_ip_stats` / `handle_get_stats` /
`handle_get_status` are pure reads.

## b) Is the gate positioned correctly in `process_request`? Bypassable?

**YES — correctly positioned, not bypassable by ordering or a second entry point.**

`src/ipc/server/handlers.rs:78-95`. Order in `process_request` is:
(1) `authorize(p, &req)` when a principal exists → 403; (2) the new
no-principal mutation gate → 401; (3) actor attribution; (4) dispatch.
Step (1) is skipped exactly when `principal` is `None`, which is precisely
the case the new gate covers — so the two checks are complementary, never
shadowing each other. There is no ordering window: the gate runs before the
`match` that reaches any handler.

Bypass surfaces checked:
- **Other entry point:** `process_request` has exactly one caller
  (`connection.rs:126`); `parse_frame` (`connection.rs:197`) is its only
  producer of `(Request, Option<Principal>)`. No second dispatcher exists.
- **Principal spoofing:** `principal` is `Some` only via
  `verify_frame_auth` → `ramshield_protocol::auth::verify_authenticated`
  (HMAC + replay store), and only when `auth_enforced` (= `!live_keys.is_empty()`,
  `connection.rs:52`). With no keys, `parse_frame` returns `principal = None`
  (`connection.rs:206-208`) → the gate fires. There is no way to obtain a
  principal without a valid HMAC.
- **Config flip at runtime:** the gate reads `engine.config.load()` live, so a
  dashboard patch can turn it off — but that requires a valid dashboard session
  and is the documented escape hatch (see e).
- **Metrics only:** `inc_rejected(1)` + `inc_ipc_auth_rejections(1)` are
  counters (both already used at `connection.rs:255-256`); no state is mutated
  on the refusal path.

## c) Is the dashboard gate correct? Is the CSRF-then-session ordering right?

**YES.**

`src/dashboard/mod.rs:348-381`. The CSRF check runs first and returns 403 for
a cross-origin request (`is_cross_origin`, `mod.rs:311-345`); only after
`inc_csrf_allowed()` does the session gate run and return 401. This ordering
is right and is asserted to be stable: the two pre-existing CSRF tests still
expect 403 and are unchanged, and the new test
`config_patch_requires_a_session_without_a_configured_password` sends no
`Origin`/`Referer` on a loopback host (the fail-open branch) and therefore
reaches the session gate.

Why 403-before-401 is the better answer: the origin is the more specific
fact, and the status must not depend on whether the caller happens to hold a
session (otherwise a session-holding cross-origin request and a
non-session-holding one would leak different status codes, and the CSRF
metric would be polluted).

Gated routes: `POST /api/config` is the only mutating dashboard route
(`mod.rs:63`). All other routes are `get` reads (`/api/snapshot`,
`/api/stream`, `/api/history/*`, `/api/blocks/active`, `/api/traffic/subnets`,
`/api/status/modules`, `/api/config` GET) plus `/`, `/healthz`, `/livez`,
`/metrics` — none of which mutate engine or enforcement state.

Single cookie implementation: `session_valid()`
(`src/dashboard/auth.rs:297-319`) is now the only cookie parser; the
`require_auth` middleware was refactored to call it
(`auth.rs:334-337`), so the two cannot drift.

## d) Are the two config defaults SAFE? (the important question)

**YES — every path that could produce a false default was checked; all yield
`true` (the secure value).**

`ipc.require_auth_for_mutations` (`crates/ramshield-config/src/sections/ipc.rs:52-54`)
and `dashboard.require_auth_for_config`
(`crates/ramshield-config/src/sections/dashboard.rs:28-30`) both use
`#[serde(default = "default_true")]`, where `default_true()` is
`crates/ramshield-config/src/lib.rs:22-28` and returns `true`. Note the
doc-comment rationale: a bare `#[serde(default)]` on a `bool` yields `false`,
which here would be the *insecure* value, hence the explicit helper.

Paths enumerated and checked:

| Path | Result |
|---|---|
| Serde, key absent in TOML/JSON | `default_true` → `true` ✓ |
| Serde, whole `[ipc]`/`[dashboard]` section absent | section `Default` impl → `true` (`ipc.rs:104`, `dashboard.rs:102`) ✓ |
| `Config::default()` (derived) | uses the section `Default` impls above → `true` ✓ |
| `Config::from_toml_file` / `Config::load` | plain `toml::from_str` + `validate`; no post-load rewrite of these fields → `true` ✓ |
| Env-var overrides (`crates/ramshield-config/src/env.rs:239-283`) | **no** env var exists for either flag, and the env layer only ever *assigns* listed fields — there is no generic "clear unknown fields" path → cannot be flipped to `false` ✓ |
| `ConfigPatch` (`src/dashboard/mod.rs:241-251`) → `cfg.ipc = v` / `cfg.dashboard = v` (`mod.rs:404-420`) | **the whole section struct is deserialized by the same serde impls**, so a patch carrying an `ipc` section that was loaded from an *older* config (no `require_auth_for_mutations` key) deserializes the field to `default_true` = `true`, and the same for `require_auth_for_config`. A patch can only set `false` if it explicitly says `require_auth_for_mutations = false` — an affirmative, operator-authored value, not an accident of omission ✓ |
| `ConfigView` → `GET /api/config` → POST-back round trip | `ConfigView` embeds the real `IpcConfig`/`DashboardConfig` structs, so the new fields round-trip with their real values; redaction only touches `auth_keys` and `admin_password_hash` (`mod.rs:214-236`) ✓ |

The dashboard gate reads the **live** config
(`state.engine.config.load().dashboard.require_auth_for_config`,
`mod.rs:375`), not the incoming patch — so a patch cannot disable the gate for
the request that carries it. Disabling it takes effect only for *later*
requests, and reaching that code at all already required a valid session.

## e) Are the escape hatches safe? Can they widen exposure past loopback?

**NO — the escape hatches cannot widen exposure past loopback.**

`crates/ramshield-config/src/validate.rs:308-400` (fail-closed public-bind
guards):
- public `ipc.tcp_addr` requires non-empty `ipc.auth_keys` **and**
  `ipc.behind_tls_proxy`; with keys present, `auth_enforced` is true
  (`connection.rs:52`), every frame carries a verified principal, and the
  role table already denies `Operator`/`Admin` actions to weak keys. The
  `require_auth_for_mutations` flag is therefore **unreachable** in a public
  bind — turning it off changes nothing.
- public `dashboard.http_addr` requires `admin_password_hash` to be set; with
  a hash set, the `require_auth` middleware already gates every `/api/*`
  route, so `require_auth_for_config` is likewise unreachable there.
- `ipc.require_auth = true` with empty `auth_keys` is rejected at startup
  (`validate.rs:395-398`, `src/ipc/server.rs:149-151`).

Consequently both escape hatches only re-open the *loopback* path they
already govern. They are also explicit-affirmative (`= false` must be typed),
documented, and each has a dedicated test proving the escape hatch works
(`ipc_wiring_unauthenticated_mutation_is_allowed_when_explicitly_enabled`,
`config_patch_is_allowed_when_the_gate_is_disabled`).

## f) Panic / deadlock / hot-path regression?

**NONE FOUND.**

- The new IPC code is straight-line: two atomic counter increments and an
  early return. No lock, no allocation on the success path, no `unwrap`/
  `expect`/indexing.
- Short-circuit ordering protects the hot path: the condition is
  `principal.is_none() && req.is_mutation() && engine.config.load()...`
  (`handlers.rs:80-82`), so for a telemetry frame (`is_mutation() == false`)
  the `ArcSwap::load()` is **never even evaluated**. The only added cost on
  the telemetry hot path is one `matches!` over the enum discriminant. In the
  authenticated case `principal.is_some()` short-circuits first.
- `engine.config.load()` is an `ArcSwap` load (lock-free read of an immutable
  snapshot) — no mutex, so no deadlock risk against `state.engine.config.store()`
  in the dashboard patch path.
- The dashboard gate adds one `session_valid()` call (a header lookup + a
  `split(';')` over the cookie header + a session-table lookup) on
  `POST /api/config` only — a cold administrative route, not a hot path.
- `session_valid()` is pure and panic-free: `to_str().ok()`, `split(';')`,
  `find_map`, `strip_prefix` — no unwrap. The refactored middleware keeps
  byte-for-byte the same logic it had before (the diff moves the same
  expression into a helper), so no behaviour change for existing sessions.
- No change to the read/telemetry handler bodies; `handle_report_connection(s)`
  and the shedding logic are untouched, so the hot ingest path is unchanged
  apart from the single `is_mutation()` call.

---

## OPTIONAL OBSERVATIONS (non-blocking)

1. **UNVERIFIED (deliberate, by instruction)**: the RED→GREEN cycles, the
   `cargo test --lib dashboard` 31/31, `cargo test --lib ipc` 17/17,
   `cargo test --test ipc_wiring` 6/6 and the 14 CI gates were not re-run by
   me; they are taken as given.
2. **Operational liveness (intentional but worth a doc line)**: in the
   no-password dashboard configuration the config patch is now permanently
   401 — a session cannot be minted without an admin password, so the only
   way to re-enable `POST /api/config` is to set
   `dashboard.require_auth_for_config = false` in `config.toml` and restart
   (or configure an admin password). This is fail-closed and correct, but it
   is a behaviour change for operators who previously used the dashboard to
   patch config in the no-credentials setup. The docs were updated; I did not
   independently re-read `docs/OPERATIONS.md` to confirm the recovery
   procedure is spelled out there.
3. **Telemetry can still indirectly cause automatic blocks.** An
   unauthenticated local process may still submit `report_connection(s)`
   frames into the detection pipeline, which can auto-block. That path is
   explicitly left open by this change (and the role table already allowed a
   `Telemetry` key to do it), so it is pre-existing behaviour, not a gap
   introduced here. It does mean "unauthenticated local process" still
   retains an indirect influence over enforcement state — only *removal* of
   mitigations is now closed. Worth a follow-up ticket, not a blocker.
4. **An authenticated dashboard admin can disable the IPC mutation gate via a
   config patch** (`ipc.require_auth_for_mutations = false` in a patch to
   `POST /api/config`, which is itself session-gated and then `validate()`d).
   This is inside an admin's authority and the patch is rejected unless it
   validates, but it does mean "dashboard session" can now relax the IPC
   control plane on a loopback bind. Consider (in a follow-up) refusing
   patches that weaken the two new flags, or at least emitting a warning
   when one is weakened.
5. **Metric skew (cosmetic)**: `inc_csrf_allowed()` is incremented before the
   session check (`mod.rs:346-350`), so a request that is later refused 401
   still counts as "CSRF allowed". The `inc_csrf_blocked` counter is
   unaffected. Moving the increment after the session gate would make the
   CSRF metric mean "passed both gates".
6. **Role-table equivalence is a proxy, not the definition.** The new test
   encodes "mutating ⇔ denied to ReadOnly but allowed to Admin". For the
   current enum that is exactly right, but it would misclassify a future
   *read-only* variant that happened to require `Admin`. A comment in
   `message.rs` noting that `is_mutation()` is the normative list and the
   role table is derived from it would keep the direction of dependency clear.
7. **`default_true` is `pub(crate)`** in `ramshield-config`; the sections
   reach it via `use crate::*`. That compiles and the derived
   `Deserialize` impl lives in the same crate, so the `pub(crate)` visibility
   is sufficient even though `IpcConfig`/`DashboardConfig` are re-exported
   and used from the `ramshield` binary crate. No action needed — noting it
   because a future move to a public helper would be a small API change.
8. Not verified by reading (out of budget, non-blocking): whether the
   updated `docs/IPC.md`, `docs/SECURITY_MODEL.md`, `docs/CONFIGURATION.md`
   and `docs/OPERATIONS.md` describe the new flags and the recovery
   procedure precisely; I read only the code diff, not the doc diff bodies.

---

REVIEWED IN 10 TOOL CALLS
