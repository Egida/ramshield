#!/usr/bin/env python3
"""Bridge RamShield's bounded FIFO to ExaBGP's text API.

ExaBGP starts this process as an API process. RamShield writes only its own
validated FlowSpec/RTBH commands to the FIFO. The bridge never evaluates shell
syntax and forwards one complete command line at a time.
"""
import os
import sys

fifo = os.environ.get("RAMSHIELD_BGP_FIFO", "/run/ramshield/flowspec.fifo")
with open(fifo, "r", encoding="ascii", errors="strict") as source:
    for line in source:
        line = line.strip()
        if not line or len(line) > 4096:
            continue
        sys.stdout.write(line + "\n")
        sys.stdout.flush()
