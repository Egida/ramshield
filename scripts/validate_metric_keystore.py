#!/usr/bin/env python3
"""Validate the versioned metric/module relationship graph."""
from __future__ import annotations

import json
import pathlib
import sys

ROOT = pathlib.Path(__file__).resolve().parents[1]
REGISTRY = ROOT / "docs/metrics/metric-keystore.json"
SCHEMA = ROOT / "docs/metrics/nosql-schema.json"


def fail(message: str) -> None:
    print(f"FAIL: {message}")
    raise SystemExit(1)


def main() -> None:
    registry = {}
    schema = {}
    try:
        registry = json.loads(REGISTRY.read_text())
        schema = json.loads(SCHEMA.read_text())
    except (OSError, json.JSONDecodeError) as exc:
        fail(f"cannot load registry: {exc}")

    for key in ("modules", "metrics", "relations", "evidence"):
        if not isinstance(registry.get(key), list):
            fail(f"{key} must be an array")
    if registry.get("version") != 1:
        fail("unsupported registry version")
    if schema.get("$id") != "https://ramshield.local/schema/metric-observability.json":
        fail("schema identity mismatch")

    modules = {m["id"] for m in registry["modules"]}
    metrics = registry["metrics"]
    metric_ids = {m["id"] for m in metrics}
    if len(metric_ids) != len(metrics):
        fail("duplicate metric ID")
    if len(modules) != len(registry["modules"]):
        fail("duplicate module ID")

    for metric in metrics:
        mid = metric["id"]
        owner = metric["owner_module"]
        if owner not in modules:
            fail(f"{mid}: unknown owner module {owner}")
        if not metric["writers"]:
            fail(f"{mid}: no writer")
        if len(metric["writers"]) != len(set(metric["writers"])):
            fail(f"{mid}: duplicate writer")
        if not metric["readers"]:
            fail(f"{mid}: no reader")
        if not metric["invariants"]:
            fail(f"{mid}: no invariant")

    relation_types = {"writes", "reads", "exports", "renders", "verifies", "derived_from", "logs"}
    for rel in registry["relations"]:
        if rel["type"] not in relation_types:
            fail(f"unknown relation type {rel['type']}")
        for endpoint in (rel["from"], rel["to"]):
            if endpoint not in modules and endpoint not in metric_ids and endpoint != "dashboard.dom":
                fail(f"relation points to unknown object: {endpoint}")

    evidence_ids = [e["id"] for e in registry["evidence"]]
    if len(evidence_ids) != len(set(evidence_ids)):
        fail("duplicate evidence ID")
    if not registry["evidence"]:
        fail("no executable evidence")

    print(f"OK: {len(modules)} modules, {len(metrics)} metrics, {len(registry['relations'])} relations, {len(registry['evidence'])} evidence records")


if __name__ == "__main__":
    main()
