# RamShield Module Documentation Standard

**Audience:** Every contributor, reviewer, and tooling that touches docs. **Contract — no exceptions.**

---

## 1. Structure per page (fixed, all 10 sections)

Every `.md` in `docs/` must contain exactly these sections in this order:

1. **Title** — `# <Page Title>`
2. **Metadata line** — `**Audience:** <who reads this> · **Last verified:** <YYYY-MM-DD> · **Gate:** <exact command that validates this page>`
3. **One-paragraph pitch** — What this page covers, no fluff, no "RamShield is a…" openers.
4. **Sections** — Numbered `## N.` headings, one blank line after each header.
5. **Tables only for referential data** — Config keys, metric names, command flags. Prose for invariants.
6. **ASCII flow** — Only `→` and `│` for data flow; no box-drawing characters.
7. **Every name is verbatim from source** — File paths, config keys, metric names, types, function signatures copied from code, never paraphrased. This kills drift.
8. **No marketing language** — Zero "powerful", "robust", "cutting-edge", "seamless".
9. **Definition of Done footer** — `# Definition of Done` checklist that every page must pass before merge.

---

## 2. Validation gates (hard, automated)

A page is **not done** until all run:

| Check | Command | What fails |
|---|---|---|
| Config keys exist | `tomlq --input config.baseline.toml --output json` + grep | Any key in the doc missing from schema |
| Metric names exact | `grep -r 'ramshield_' crates/ramshield-metrics/src/lib.rs` | Any metric name in doc not in source |
| File paths exist | `ls -1 <path>` | Dead relative links |
| Commands run | `cargo check --all-targets --all-features` | Broken example commands |
| Link check | `markdown-link-check docs/*.md` | Dead internal/external links |

---

## 3. Prohibited patterns

| Pattern | Why |
|---|---|
| "RamShield is a…" / "This module handles…" | State the function, not the noun |
| "We use X to…" | Second person ("You use X…") or imperative |
| Config keys without `section.key` | Ambiguous; use `[section].key` always |
| Metric names without `ramshield_` prefix | The exported names are literal |
| "TODO" / "FIXME" in merged docs | Fix before merge or it's not done |
| Version numbers not from `Cargo.toml` | Drift |

---

## 4. Naming conventions

- Config keys: `[section].key_name` (e.g., `[detection].rps_threshold`)
- Metric names: `ramshield_snake_case` (exact export)
- Binary names: backticks `ramshield` `ramshield-cli`
- Crate names: `ramshield-<name>` (hyphen)
- Module paths: `src/<module>/mod.rs` or `crates/<crate>/src/lib.rs`

---

## 5. Definition of Done (every page)

- [ ] Title + audience + gate line present
- [ ] All config keys verified against `config.baseline.toml` schema
- [ ] All metric names verified against `ramshield-metrics/src/lib.rs` exports
- [ ] All file paths exist on disk
- [ ] All example commands execute without error
- [ ] `markdown-link-check` passes
- [ ] `cargo check --all-targets --all-features` passes (code blocks compile)
- [ ] No prohibited patterns
- [ ] Freshness header date updated

---

## 6. Enforcement

The `scripts/docs_verify.sh` (to be added) runs the above gates in CI. A page failing any gate blocks merge.

**This standard is not aspirational — it is the contract.** If a page in `docs/` violates it, the page is broken, not the standard.