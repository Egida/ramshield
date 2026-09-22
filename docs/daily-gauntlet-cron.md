# Daily gauntlet cron (feature)

A scheduled Hermes agent job runs one attack-gauntlet round per day
against live prod and writes a dated report into this branch. This file
is the feature spec: how the job is defined, where its reports live,
and how to recreate it if the Hermes cron state is lost.

## Job

- **Name:** `ramshield-daily-gauntlet`
- **Hermes job id:** `931321881e06`
- **Schedule:** every day at 10:00 (local)
- **Skill:** `ramshield-development` (loads
  `references/attack-gauntlet.md`: harness, wire contract, baselines,
  pitfalls)
- **Workdir:** this repo
- **Continuity:** on — each run sees its own previous output, so
  shapes never repeat across days
- **Delivery:** local (output saved in Hermes cron history; no chat
  target)

## Behavior per run

1. **Pre-flight.** Prod alive check (`pgrep -x ramshield`); if down,
   relaunch the deployed binary only and wait for the `XDP dataplane
   active` log line — missing caps/XDP means abort with a report, no
   attack. Harness (`/tmp/atk/{lib,driver,sampler}.py`) rebuilt from
   the skill's harness section if rebooted away.
2. **Ten handwritten shapes.** 7–8 parameterized re-checks of known
   classes with different sizes/rates than previous days, 2–3 fresh
   probes (candidate thin classes live in
   `ATTACK_GAUNTLEND_REMAINING.md` §3). Restart-under-load is
   deliberately excluded — it stays a manual test.
3. **Safety.** RFC5737/RFC3849 reserved attack sources only; signed
   IPC only (HMAC keys exist solely in untracked `config.prod.toml` —
   never printed or committed); exact-PID kills only, no `pkill -f`;
   no `sudo`, no rebuilds (setcap caps die on relink — a needed
   rebuild is flagged in the report, not done); config mutation under
   the D6 rules only (merge-key-free bodies, tripwire +
   verify-with-restore); single WAL writer (prod owns
   `/tmp/ramshield_wal`); total runtime cap 45 min; teardown stops
   load generators first.
4. **Report.** `ATTACK_GAUNTLEND_DAILY/YYYY-MM-DD.md` on this branch
   (per-round table, anomalies, flagged defects — flagged only, no
   product fixes from this path), committed here and pushed
   (`git push origin daily-gauntlet`). Master is not touched by the
   job.

## Report layout

```
ATTACK_GAUNTLEND_DAILY/
  2026-09-23.md
  2026-09-24.md
  ...
```

Each report: prod health line (pid, XDP mode, errs, RSS), 10-row
shape/number/verdict table, anomalies, defects flagged for manual
follow-up.

## Recreation

If the Hermes cron state is lost, recreate via
`cronjob_manage(action='create')` with: `name` above, schedule
`every day at 10am`, `skills: ["ramshield-development"]`,
`workdir` = repo root, `continuity: true`, `deliver: "local"`, and the
prompt below (kept verbatim; the job's own prompt is the source of
truth — if this file and the live job disagree, update this file).

### Prompt (verbatim)

> Daily RamShield attack-gauntlet round: author and run EXACTLY 10
> handwritten test shapes against live prod, one at a time, improvised
> from live metrics, and record results. This is drift-detection, not
> new-class discovery.
>
> READ FIRST: the ramshield-development skill's
> references/attack-gauntlet.md (harness, wire contract, round
> baselines, pitfalls) and the repo docs ATTACK_GAUNTLEND_R1/R2/R3.md
> and ATTACK_GAUNTLEND_REMAINING.md.
>
> PRE-FLIGHT:
> 1. Prod check: pgrep -x ramshield. If down: relaunch ONLY the
>    deployed binary with
>    '/home/m/vehicle_of_rationalism/ramshield/beta/rs/target/release/ramshield
>    --config /home/m/vehicle_of_rationalism/ramshield/beta/rs/config.prod.toml'
>    (log append to /tmp/ramshield_prod_run.log), verify an
>    'XDP dataplane active' line appears; note the relaunch in the
>    report. If caps/XDP missing: STOP, report it, do not attack.
> 2. Harness check: /tmp/atk/{lib.py,driver.py,sampler.py} may be gone
>    after reboot; rebuild from the skill's harness section. If prod
>    is healthy, ensure sampler + driver are running (background,
>    exact PID tracking).
>
> ROUNDS (exactly 10, handwritten, new files in /tmp/atk/queue/
> continuing the highest existing number):
> - 7-8 parameterized re-checks of known classes (different
>   sizes/rates/shape than recent daily reports — use the continuity
>   context to avoid duplicates): e.g. block/unblock rates, CIDR LPM
>   nesting, detection timing, TTL semantics, IPC desync/replay.
> - 2-3 fresh probes: one truly-new angle per day if found (consult
>   ATTACK_GAUNTLEND_REMAINING.md §3 for candidate thin classes); the
>   rest = your own judgment from live metrics.
> - Do NOT run restart-under-load (SIGKILL prod) — that stays manual.
>
> SAFETY (non-negotiable):
> - Attack sources ONLY RFC5737/RFC3849 reserved ranges
>   (198.51.100/24, 203.0.113/24, 192.0.2/24, 2001:db8::/32).
> - IPC wire: tag 'type' not 'op'; block_ip {ip, reason, ttl_secs}
>   reason REQUIRED; check via check_ip; get_stats is lightweight.
>   Unsigned frames are 401 — always sign via lib.py.
> - NEVER print or commit key material; keys live only in untracked
>   config.prod.toml.
> - Kill processes by EXACT PID only. NEVER pkill -f a pattern that
>   can match your own shell. Use STOP sentinels for harness
>   teardown.
> - No sudo, no cargo build/rebuild (setcap caps die on relink). If a
>   fix needs a rebuild: flag it in the report, do not build.
> - Config mutation: D6 rules only — merge-key-free bodies, tripwire +
>   verify-with-restore; no section-replace bodies.
> - Single WAL writer: prod owns /tmp/ramshield_wal; never open a
>   second process on it.
> - Total campaign runtime cap: 45 minutes. Teardown: stop
>   soak/load generators first, verify with ps, then report.
>
> METRICS: /metrics has a 1s render cache — sample counters across
> >=1s boundaries; truth = prometheus counters + check_ip, not
> response-object booleans.
>
> OUTPUT:
> - Work on the `daily-gauntlet` branch (this is the branch the job
>   owns): if the repo working tree is on another branch,
>   `git checkout daily-gauntlet` first (never commit reports to
>   master).
> - Write repo doc ATTACK_GAUNTLEND_DAILY/YYYY-MM-DD.md: per-round
>   table (shape, load, key numbers, verdict), anomalies, any defect
>   flagged (no product fixes on this path — flag only).
> - git add the doc + commit + `git push origin daily-gauntlet`
>   (explicit push of the branch).
> - FINAL RESPONSE: one-line prod health line (pid, XDP, errs, RSS),
>   10-row compact table, list of anomalies. Keep it under ~40 lines.
