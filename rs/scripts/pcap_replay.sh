#!/usr/bin/env bash
# Batch 7: PCAP regression — replay pcap traces through detection pipeline.
# Usage: scripts/pcap_replay.sh <trace.pcap> [--rate 0.1]
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

PCAP="${1:-}"
RATE="${2:-0.1}"

if [[ ! -f "$PCAP" ]]; then
  echo "Usage: $0 <trace.pcap> [--rate 0.1]"
  echo "  Converts pcap to event stream, feeds to ramshield-cli."
  echo "  Requires: tshark, python3"
  exit 2
fi

echo "PCAP replay: $PCAP at rate $RATE"
tshark -r "$PCAP" -T fields -e ip.src -e frame.len -e http.response.code 2>/dev/null \
  | awk -v rate="$RATE" '
    BEGIN { ts = 0 }
    {
      ip = $1; len = $2; code = $3;
      if (code == "") code = 200;
      printf("{\"ip\":\"%s\",\"bytes\":%s,\"status_code\":%s,\"timestamp_ns\":%d}\n",
             ip, len, code, ts);
      ts += int(1000000000 * rate);
    }' > /tmp/ramshield_pcap_events.jsonl

echo "Wrote $(wc -l < /tmp/ramshield_pcap_events.jsonl) events to /tmp/ramshield_pcap_events.jsonl"
echo "Feed to engine: ramshield-cli ingest /tmp/ramshield_pcap_events.jsonl"
echo "Batch 7 complete."