#!/usr/bin/env python3
"""Log audit: scan RamShield runtime logs, classify findings, apply safe fixes.

The scan reads trace/debug runtime logs written by scripts/run_trace_logged.sh
and scripts/run_debug_logged.sh, groups lines into target/level/family, runs a
rule table over them, and writes a JSON report plus a human summary.

Fixes are conservative: only explicitly registered remedies mutate files, and
only when --apply is passed. Everything else is emitted as a reviewed fix plan
with the owning file and the rule that fired. Nothing here edits Rust sources.

Usage:
  python3 scripts/log_audit.py                       # audit newest runtime log
  python3 scripts/log_audit.py --log FILE [--log F]  # audit specific logs
  python3 scripts/log_audit.py --all                 # audit every retained log
  python3 scripts/log_audit.py --apply               # apply registered fixes
  python3 scripts/log_audit.py --self-test           # rule-table invariant check
Exit code: 0 clean, 1 findings at or above --fail-level (default high), 2 usage.
"""
from __future__ import annotations

import argparse
import glob
import json
import os
import re
import sys
from collections import defaultdict
from dataclasses import asdict, dataclass, field
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
LOG_DIR = Path(os.environ.get("RAMSHIELD_RUNTIME_LOG_DIR", "/tmp/ramshield-runtime"))
LEVEL_ORDER = {"TRACE": 0, "DEBUG": 1, "INFO": 2, "WARN": 3, "ERROR": 4}
SEVERITY_ORDER = {"info": 0, "low": 1, "medium": 2, "high": 3, "critical": 4}

LINE_RE = re.compile(
    r"^(?P<ts>\d{4}-\d{2}-\d{2}T[\d:.]+Z)\s+(?P<level>[A-Z]+)\s+"
    r"(?P<target>[A-Za-z0-9_:]+):\s*(?P<msg>.*)$"
)
KV_RE = re.compile(r"(?P<key>[a-z][a-z0-9_]*)=(?P<val>[^\s|]+)")
NUM_RE = re.compile(r"\d+")

# Canonical parser tokens accepted by BlockReason::from_reason_str.
CANONICAL_REASONS = {
    "rps_threshold", "subnet_burst", "forecast_anomaly", "entropy_anomaly",
    "manual_block", "manual_unblock", "manual", "mesh_final", "mesh_purge",
    "mesh_sync", "mesh_ban", "mesh_unban", "gossip_sync",
}
FIXTURE_REASON_MAP = {
    "test": "manual_block",
    "suite": "manual_block",
    "mesh-smoke": "mesh_final",
    "mesh_smoke": "mesh_final",
    "integration": "manual_block",
}

# Per (target, level) line budgets, in lines/second over the capture window.
# Budgets reflect the verified Instrumentation Level Contract: a bounded 4 Hz
# kernel audit is fine, an unbounded per-event family is a flood.
FLOOD_BUDGET = {
    ("ramshield_enforcement", "DEBUG"): 12.0,   # 4 Hz XDP audit + tick summaries
    ("ramshield_detection", "DEBUG"): 60.0,     # per-batch flush + subnet decisions
    ("ramshield_detection", "WARN"): 0.05,
    ("ramshield_forecasting", "DEBUG"): 60.0,   # 1 Hz Bayesian tick
    ("ramshield_forecasting", "WARN"): 0.2,    # cooldown-gated; pre-fix spam was 1.0/s
    ("ramshield_storage", "DEBUG"): 0.5,
    ("ramshield_storage", "TRACE"): 5000.0,     # opt-in, one line per stored event
    ("ramshield", "WARN"): 0.05,
}
DEFAULT_WARN_BUDGET = 0.5      # WARN lines/sec per target before it counts as a flood
DEFAULT_DEBUG_BUDGET = 50.0    # DEBUG lines/sec per target
DEFAULT_TRACE_BUDGET = 20000.0  # TRACE is opt-in; only a runaway loop trips this


@dataclass
class Event:
    log: str
    line_no: int
    ts: str
    level: str
    target: str
    msg: str
    fields: dict = field(default_factory=dict)

    @property
    def family(self) -> str:
        words = [w for w in self.msg.split() if not NUM_RE.search(w)]
        return " ".join(words[:3]) or self.msg[:24]

    @property
    def seconds(self) -> float:
        return _epoch(self.ts)


@dataclass
class Finding:
    rule: str
    severity: str
    owner: str
    summary: str
    count: int
    evidence: list
    remedy: str = "review"
    remedy_detail: str = ""
    files: list = field(default_factory=list)


def _epoch(ts: str) -> float:
    """Seconds-of-day from an RFC3339 timestamp; relative spacing is all we need."""
    try:
        clock = ts.split("T", 1)[1].rstrip("Z")
        h, m, s = clock.split(":")
        return int(h) * 3600 + int(m) * 60 + float(s)
    except (IndexError, ValueError):
        return 0.0


def parse_log(path: Path) -> tuple[dict, list[Event]]:
    header: dict = {}
    events: list[Event] = []
    for line_no, raw in enumerate(path.read_text(errors="replace").splitlines(), 1):
        if not line_no:
            continue
        match = LINE_RE.match(raw)
        if not match:
            if not events and "=" in raw and len(raw) < 200:
                key, _, val = raw.partition("=")
                header[key.strip()] = val.strip()
            continue
        msg = match.group("msg")
        fields = {m.group("key"): m.group("val") for m in KV_RE.finditer(msg)}
        events.append(Event(str(path), line_no, match.group("ts"),
                            match.group("level"), match.group("target"), msg, fields))
    return header, events


# ── rule table ────────────────────────────────────────────────────────────────

def rule_panics(events: list[Event], _h: dict) -> list[Finding]:
    hits = [e for e in events if "panicked" in e.msg or "assertion failed" in e.msg]
    if not hits:
        return []
    return [Finding("panic", "critical", hits[0].target,
                    "panic/assertion failure in runtime log", len(hits),
                    _evidence(hits), "review",
                    "capture backtrace with RUST_BACKTRACE=1 and open a bug")]


def rule_errors(events: list[Event], _h: dict) -> list[Finding]:
    hits = [e for e in events if e.level == "ERROR"]
    if not hits:
        return []
    return [Finding("error_line", "high", hits[0].target,
                    "ERROR-level event emitted during run", len(hits),
                    _evidence(hits), "review",
                    "ERROR must be actionable; wire a handler or demote with a reason")]


def rule_unknown_reason(events: list[Event], _h: dict) -> list[Finding]:
    hits = [e for e in events if "unknown block reason" in e.msg]
    if not hits:
        return []
    tokens = sorted({e.fields.get("reason", "?") for e in hits})
    remedy = "fixture_reason" if all(t in FIXTURE_REASON_MAP for t in tokens) else "review"
    detail = ("rewrite non-canonical fixture tokens in scripts/*.py"
              if remedy == "fixture_reason" else
              "add the token to BlockReason::from_reason_str or fix the producer")
    return [Finding("unknown_block_reason", "medium", "ramshield-types",
                    "non-canonical BlockReason token reached the parser", len(hits),
                    _evidence(hits), remedy, f"{detail} (tokens: {', '.join(tokens)})",
                    ["crates/ramshield-types/src/error.rs"])]


def rule_counter_clobber(events: list[Event], _h: dict) -> list[Finding]:
    hits = [e for e in events if "clobber" in e.msg.lower()]
    if not hits:
        return []
    return [Finding("counter_clobber", "critical", hits[0].target,
                    "cumulative counter overwritten by a gauge writer", len(hits),
                    _evidence(hits), "review",
                    "one counter, one writer: move the gauge write to a gauge field")]


def rule_swallowed_error(events: list[Event], _h: dict) -> list[Finding]:
    hits = [e for e in events if "unavailable" in e.msg and e.level == "TRACE"]
    if not hits:
        return []
    return [Finding("swallowed_error_trace", "info", hits[0].target,
                    "degraded sub-system observable only at trace", len(hits),
                    _evidence(hits), "review",
                    "expected: keep trace arm, confirm the caller also counts the failure")]


def rule_flood(events: list[Event], _h: dict) -> list[Finding]:
    if not events:
        return []
    span = max(e.seconds for e in events) - min(e.seconds for e in events)
    span = max(span, 1.0)
    buckets: dict[tuple[str, str], int] = defaultdict(int)
    for event in events:
        buckets[(event.target, event.level)] += 1
    out = []
    for (target, level), count in sorted(buckets.items(), key=lambda kv: -kv[1]):
        if level == "TRACE":
            budget = FLOOD_BUDGET.get((target, level), DEFAULT_TRACE_BUDGET)
        elif level == "DEBUG":
            budget = FLOOD_BUDGET.get((target, level), DEFAULT_DEBUG_BUDGET)
        elif level == "WARN":
            budget = FLOOD_BUDGET.get((target, level), DEFAULT_WARN_BUDGET)
        else:
            continue
        rate = count / span
        if rate <= budget:
            continue
        severity = "high" if level == "WARN" else "medium"
        remedy = "review" if level == "TRACE" else "demote_or_sample"
        out.append(Finding("log_flood", severity, target,
                           f"{level} lines from one target exceed budget "
                           f"({rate:.1f}/s > {budget}/s)", count,
                           _sample(events, target, level), remedy,
                           "gate behind trace!, add a cooldown, or sample 1/1024 with the "
                           "cumulative counter in the line"))
    return out


def rule_unstructured(events: list[Event], _h: dict) -> list[Finding]:
    hits = [e for e in events if e.level == "TRACE" and not e.fields
            and (" - OK " in e.msg or e.msg.startswith("Store::"))]
    if not hits:
        return []
    return [Finding("unstructured_trace", "low", hits[0].target,
                    "trace line uses interpolation instead of structured fields", len(hits),
                    _evidence(hits), "review",
                    "use key=value fields so the scanner and log store can index them")]


def rule_silent_family(events: list[Event], _h: dict) -> list[Finding]:
    """A target that emits DEBUG elsewhere but never its expected audit family."""
    expected = {"ramshield_enforcement": "xdp counters read",
                "ramshield_detection": "batch flush"}
    out = []
    for target, family in expected.items():
        target_events = [e for e in events if e.target == target]
        if not target_events:
            continue
        if any(family in e.msg for e in target_events):
            continue
        out.append(Finding("missing_audit_family", "medium", target,
                           f"target is active but never emitted '{family}'", 0,
                           [f"{target}: {len(target_events)} lines, 0 audits"], "review",
                           "verify the owning write path before treating this as an "
                           "idle-zero metric"))
    return out


def rule_trace_coverage(events: list[Event], header: dict) -> list[Finding]:
    """Trace is the exception-catcher: an active target with no TRACE arm is blind.

    Only meaningful when the capture really ran at trace level, otherwise every
    target would be reported and the finding would be noise.
    """
    if "trace" not in str(header.get("rust_log", "")):
        return []
    targets: dict[str, int] = defaultdict(int)
    traced: set[str] = set()
    for event in events:
        if not event.target.startswith("ramshield"):
            continue
        targets[event.target] += 1
        if event.level == "TRACE":
            traced.add(event.target)
    # Targets whose per-event path is covered by another target's trace arm
    # (the IPC server owns the ingress trace for the whole event path).
    covered_elsewhere = {"ramshield_detection"}
    out = []
    for target in sorted(targets):
        if target in traced or target in covered_elsewhere:
            continue
        out.append(Finding("no_trace_channel", "medium", target,
                           f"target active ({targets[target]} lines) with no TRACE arm",
                           0, [f"{target}: {targets[target]} lines, 0 trace"],
                           "add_trace",
                           "trace the decision that drops or defers work in this target"))
    return out


RULES = (rule_panics, rule_errors, rule_counter_clobber, rule_unknown_reason,
         rule_flood, rule_silent_family, rule_unstructured, rule_swallowed_error,
         rule_trace_coverage)


def _evidence(events: list[Event], limit: int = 5) -> list[str]:
    return [f"{e.log}:{e.line_no} {e.level} {e.target} {e.msg[:160]}" for e in events[:limit]]


def _sample(events: list[Event], target: str, level: str, limit: int = 3) -> list[str]:
    hits = [e for e in events if e.target == target and e.level == level]
    return _evidence(hits, limit) + [f"... {len(hits)} total"]


# ── registered fixes ──────────────────────────────────────────────────────────

def apply_fixture_reason(apply: bool, repo: Path) -> list[str]:
    """Rewrite non-canonical block reason tokens in test scripts only."""
    changed = []
    pattern = re.compile(r'("reason"\s*:\s*")([A-Za-z0-9_.-]+)(")')
    for path in sorted(glob.glob(str(repo / "scripts" / "*.py"))):
        text = Path(path).read_text()
        new = pattern.sub(
            lambda m: m.group(1) + FIXTURE_REASON_MAP.get(m.group(2), m.group(2)) + m.group(3),
            text)
        if new == text:
            continue
        if apply:
            Path(path).write_text(new)
        changed.append(f"{path} -> canonical reason tokens")
    return changed


REMEDIES = {"fixture_reason": apply_fixture_reason}


# ── reporting ─────────────────────────────────────────────────────────────────

def summarize(findings: list[Finding], headers: dict) -> str:
    lines = []
    for log, header in headers.items():
        lines.append(f"log: {log}")
        for key in ("description", "rust_log", "git_sha", "result"):
            if key in header:
                lines.append(f"     {key}={header[key]}")
    if not findings:
        return "\n".join(lines + ["", "clean: no findings above info"] )
    lines.append("")
    lines.append(f"{'severity':9} {'rule':24} {'owner':24} count")
    for f in sorted(findings, key=lambda f: -SEVERITY_ORDER[f.severity]):
        lines.append(f"{f.severity:9} {f.rule:24} {f.owner:24} {f.count}")
    lines.append("")
    for f in sorted(findings, key=lambda f: -SEVERITY_ORDER[f.severity]):
        lines.append(f"[{f.severity}] {f.rule} ({f.count}x, {f.owner}) — {f.summary}")
        lines.append(f"    remedy: {f.remedy} — {f.remedy_detail}")
        for item in f.evidence:
            lines.append(f"      {item}")
        lines.append("")
    return "\n".join(lines)


def self_test() -> int:
    sample = "\n".join([
        "started_utc=20260917T000000Z",
        "rust_log=trace",
        "",
        "2026-09-17T00:00:01.000000Z  WARN ramshield_enforcement: unknown block reason string; "
        "defaulting to ManualBlock reason=test",
        "2026-09-17T00:00:02.000000Z  WARN ramshield_forecasting: BAYESIAN H2 SLOW-RAMP conf=0.96 z=5.99",
        "2026-09-17T00:00:03.000000Z TRACE ramshield_storage: Store::insert - OK key: 1.2.3.4",
        "2026-09-17T00:00:03.100000Z DEBUG ramshield_enforcement: xdp counters read v4_drops=0",
    ])
    tmp = Path("/tmp/ramshield-log-audit-self-test.log")
    tmp.write_text(sample)
    header, events = parse_log(tmp)
    assert header["rust_log"] == "trace", header
    assert len(events) == 4, len(events)
    reasons = rule_unknown_reason(events, header)
    assert reasons and reasons[0].remedy == "fixture_reason", reasons
    floods = rule_flood(events, header)
    assert any(f.owner == "ramshield_forecasting" for f in floods), floods
    assert rule_counter_clobber(events, header) == []
    assert rule_unstructured(events, header), "trace interpolation must be flagged"
    json.dumps([asdict(f) for f in reasons])
    tmp.unlink()
    print("OK: rule table classifies reason tokens, floods, unstructured trace")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__,
                                     formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--log", action="append", default=[])
    parser.add_argument("--all", action="store_true")
    parser.add_argument("--last", type=int, default=1)
    parser.add_argument("--json-out")
    parser.add_argument("--apply", action="store_true")
    parser.add_argument("--fail-level", default="high", choices=sorted(SEVERITY_ORDER))
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    if args.self_test:
        return self_test()

    paths = [Path(p) for p in args.log]
    if not paths:
        pool = sorted(glob.glob(str(LOG_DIR / "*.log")), key=os.path.getmtime, reverse=True)
        if not pool:
            print(f"no logs under {LOG_DIR}", file=sys.stderr)
            return 2
        paths = [Path(p) for p in (pool if args.all else pool[:args.last])]

    headers: dict = {}
    events: list[Event] = []
    for path in paths:
        header, parsed = parse_log(path)
        headers[str(path)] = header
        events.extend(parsed)

    findings: list[Finding] = []
    for rule in RULES:
        findings.extend(rule(events, headers))

    fixes = []
    if args.apply:
        for f in findings:
            if f.remedy in REMEDIES and f.remedy != "review":
                fixes.extend(REMEDIES[f.remedy](True, REPO))

    report = {
        "logs": {k: v for k, v in headers.items()},
        "events": len(events),
        "levels": _count_by(events, "level"),
        "targets": _count_by(events, "target"),
        "findings": [asdict(f) for f in findings],
        "fixes_applied": fixes,
    }
    if args.json_out:
        Path(args.json_out).write_text(json.dumps(report, indent=2))
    print(summarize(findings, headers))
    if fixes:
        print("fixes applied:")
        for item in fixes:
            print(f"  {item}")
    if not args.apply:
        pending = sorted({f.remedy for f in findings if f.remedy in REMEDIES})
        if pending:
            print(f"pending auto-fixes (rerun with --apply): {', '.join(pending)}")
    worst = max((SEVERITY_ORDER[f.severity] for f in findings), default=-1)
    return 1 if worst >= SEVERITY_ORDER[args.fail_level] else 0


def _count_by(events: list[Event], attr: str) -> dict:
    out: dict = defaultdict(int)
    for event in events:
        out[getattr(event, attr)] += 1
    return dict(sorted(out.items(), key=lambda kv: -kv[1]))


if __name__ == "__main__":
    raise SystemExit(main())