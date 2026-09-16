#!/usr/bin/env python3
"""Export the metric graph as import-friendly JSONL documents."""
from __future__ import annotations

import argparse
import json
import pathlib

ROOT = pathlib.Path(__file__).resolve().parents[1]
DEFAULT_INPUT = ROOT / "docs/metrics/metric-keystore.json"
DEFAULT_OUTPUT = ROOT / "docs/metrics/metric-keystore.jsonl"


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("-i", "--input", type=pathlib.Path, default=DEFAULT_INPUT)
    parser.add_argument("-o", "--output", type=pathlib.Path, default=DEFAULT_OUTPUT)
    args = parser.parse_args()
    data = json.loads(args.input.read_text())
    docs = []
    for module in data["modules"]:
        docs.append({"collection": "modules", **module})
    for metric in data["metrics"]:
        docs.append({"collection": "metrics", **metric})
    for relation in data["relations"]:
        docs.append({"collection": "relations", **relation})
    for evidence in data["evidence"]:
        docs.append({"collection": "evidence", **evidence})
    args.output.write_text("".join(json.dumps(doc, sort_keys=True) + "\n" for doc in docs))
    print(f"OK: wrote {len(docs)} JSONL documents to {args.output}")


if __name__ == "__main__":
    main()
