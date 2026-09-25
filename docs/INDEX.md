# RamShield Documentation Index

**Audience:** Anyone navigating this repo. **Last verified:** 2026-09-26 (P1–P8 code sync) · **Gate:** `ls docs/ && markdown-link-check docs/*.md`

---

This is the authoritative map of every documentation page in `docs/`. Each entry shows the file, its purpose, and its freshness status.

## Docs/ Pages

| File | Purpose | Freshness |
|---|---|---|
| `ARCHITECTURE.md` | System design, data flow, threat model | ⚠️ **Needs rewrite** — to be created |
| `FEATURES.md` | Every capability + how it works | ✅ **Current** — verified 2026-09-26 (P1–P8) |
| `TUNING.md` | Every config key: default, range, trade-off, when to change | ✅ **Current** — verified 2026-09-26 (`behind_tls_proxy`, `key_roles`) |
| `IPC.md` | IPC + HMAC auth + KeyRole wire contract | ✅ **Current** — verified 2026-09-26 |
| `OBSERVABILITY.md` | Metrics catalog, logs, dashboard, SSE | ⚠️ **Needs rewrite** — to be created |
| `RUNBOOK.md` | Operations: start/stop, XDP attach, WAL, backup | ⚠️ **Needs rewrite** — to be created |
| `PROTOCOL.md` | IPC + auth wire contract | ⚠️ **Needs rewrite** — merge from `docs/IPC.md` |
| `RELEASE.md` | Build, smoke, release process | ⚠️ **Needs rewrite** — merge from `docs/PRODUCTION_RELEASE_PROCESS.md` |
| `ROADMAP.md` | Dated milestones with verifiable outcomes | ✅ **Current** — exists |
| `LIMITATIONS.md` | Honest boundaries (readiness ledger) | ⚠️ **Needs rewrite** — merge from `docs/PRODUCTION_READINESS.md` |
| `PRODUCTION_READINESS.md` | Readiness ledger | ✅ **Current** — Phase 1 closed 2026-09-26 |

---

## Root Pages

| File | Purpose | Freshness |
|---|---|---|
| `README.md` | Front door — what, why, quick start, CLI, IPC, config, performance | ✅ **Current** — verified 2026-09-26 |
| `DOC_STANDARD.md` | The contract every doc must satisfy | ✅ **Current** — verified 2026-09-25 |
| `CONTRIBUTING.md` | How to contribute, code standards, commit messages | ✅ **Current** — exists |
| `SECURITY.md` | Vulnerability reporting, disclosure policy | ✅ **Current** — exists |
| `CHANGELOG.md` | Release history | ✅ **Current** — P1–P8 in `[Unreleased]` |
| `COMPLIANCE_*.md` | Audit artifacts (CERA, RODO, KSEF, PKE) | ⚠️ **Audit-specific** — not user docs |

---

## Freshness Legend

| Badge | Meaning |
|---|---|
| ✅ **Current** | Passes all `DOC_STANDARD.md` gates; source-verified |
| ⚠️ **Needs rewrite** | Either missing or stale; must be rewritten to the new standard |

---

## Definition of Done (for this index)

- [x] Operator-facing IPC/auth/config pages match P1–P8 code
- [ ] Every `docs/*.md` page listed
- [ ] Freshness badge reflects actual `DOC_STANDARD.md` gate status
- [ ] No dead relative links (`markdown-link-check docs/INDEX.md` passes)
- [ ] New pages added here when created
