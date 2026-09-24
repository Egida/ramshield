# RamShield — what remains on the table after the attack gauntlet

After ~32 hand-authored rounds (A1–A10, B1–B6, C1–C3, D1–D11), the
gauntlet's verdict is stable: the dataplane is non-blocking by
construction, every failure class is metered, and the only defects
found across the whole campaign were in the harness's own counter and
message semantics. What follows is not a defect list — it is the
deliberate remainder, sorted by who owes the next move.

## 1. Design trade-offs, accepted and documented (revisit on trigger)

- **Detection head-of-line under enforcement contention** (~10×:
  199 ms clean → 2,049 ms at 32 writers, C2). Degradation, not a
  ceiling. The candidate fix — separate/priority workers for the
  enforcement path — exists as an idea but was never needed by a
  customer-shaped requirement. Trigger: any real deployment where
  enforcement and detection run at sustained max simultaneously.
- **Deterministic tiers always beat the forecast for concentrated
  attacks** (D8). This is the layered design working: `high_rps` and
  the subnet tier are fast per-IP/subnet gates; the Bayesian forecaster
  is the slow preemptive backstop for distributed ramps (proven firing
  in D8b). Open only if we ever want forecast-first semantics — today
  we do not.
- **Count-based detection, no event-level dedup** (D10). Correct for a
  single trust domain (key-holders can already fabricate events).
  Becomes a requirement the moment reporting is ever cross-trust /
  multi-tenant.
- **Section-replace config + fail-open no-Origin CSRF** (D6). Safe
  while the dashboard binds loopback only. Hardening (auth gate on the
  route + strict-origin CSRF) is required before any non-loopback
  exposure — recorded as a precondition, not a bug.

## 2. Operational facts to keep, or decide (no code needed)

- **Startup re-enforcement window ≈ 18 s** (WAL replay ≈ 9 s for
  1.65 M entries, then XDP attach; D11). Replay is idempotent and
  durable (8/8 last storm blocks survived SIGKILL). Possible future
  improvement if fast restarts ever matter: attach XDP before replay,
  or periodic state snapshots to shrink the replay set.
- **WAL rotation never observed under load** (44 B/block, flat curve,
  D3/B6). The rotation path is exercised in unit tests but never under
  a real sustained storm — acceptable, noted honestly.
- **Old IPC auth key sits in public git history — formally accepted.**
  The key was rotated on 2026-09-22 (commit `6e92c2c`) and the
  key-bearing config files (`config.bench.toml`, `config.prod.toml`)
  were removed from tracking in the same commit. The decision is to
  leave the history intact: the key is dead, the branch is private,
  and any rewrite would invalidate every existing clone, tag, and
  remote ref (`master`, `daily-gauntlet`, `v0.2.0`) for a nil risk
  reduction. This is a documented accepted trade, not a defect.
- **Launch/promo assets stay local-only until explicit go** — HN and
  Docker Hub accounts do not exist yet; crates.io publish unstarted.
- **Cron hygiene**: the RamShield operator code re-enables paused cron
  jobs (watchdog, every 6 h). Any paused jobs must stay paused on
  purpose; audit after every operator run.

## 3. Coverage gaps, deliberately left thin (low information value)

- Bloom decay under days-long churn (self-heal exists from the field
  day; needs multi-hour churn to measure).
- XDP LPM ENOSPC saturation (176k adds, 0 failures; true cap needs
  millions of adds — not worth laptop hours).
- Multi-day RSS drift / soak stability (slots were ≤ 1 h by design).
- v6 reason-string attribution names the /32 ancestor instead of the
  longest /64 (D2, cosmetic, userspace string only).

## Closed by the gauntlet (not coming back to these)

Channel capacity (C3: drains ≥ 45k eps, zero rejects), WAL cost (D3:
not durability-bound), slowloris (D4: threshold unreachable from one
host), TTL semantics (D7: 0 = permanent, >1 yr typed-rejected),
replay protection (D9: live), IPC desync/pipelining (D9: 0 wrong),
config placeholder rejection (D6b: P2 holds), CIDR LPM semantics (D1),
IPv6 path (D2), durability under SIGKILL (D11).

## Standing next step

Daily hand-authored gauntlet round (10 shapes/day) now runs as a cron
job against live prod — see `ATTACK_GAUNTLEND_DAILY/` for dated
reports. Its job is drift detection, not new-class discovery; the
remaining genuinely-new classes are thin by design (see §3).
