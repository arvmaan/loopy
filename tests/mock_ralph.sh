#!/usr/bin/env bash
# Mock Ralph: writes JSONL events to simulate a Ralph loop.
# Usage: mock_ralph.sh run -a --no-tui -q [--completion-promise TOPIC]
#
# Env vars:
#   MOCK_RALPH_EVENTS_DIR  — where to write events (default: .ralph)
#   MOCK_RALPH_LOOP_ID     — loop identifier (default: mock-loop-1)
#   MOCK_RALPH_EXIT_CODE   — exit status (default: 0)
#   MOCK_RALPH_DELAY_MS    — delay before writing completion (default: 50)

set -e

COMPLETION_PROMISE=""
while [[ $# -gt 0 ]]; do
    case "$1" in
        --completion-promise)
            COMPLETION_PROMISE="$2"
            shift 2
            ;;
        *)
            shift
            ;;
    esac
done

EVENTS_DIR="${MOCK_RALPH_EVENTS_DIR:-.ralph}"
LOOP_ID="${MOCK_RALPH_LOOP_ID:-mock-loop-1}"
EXIT_CODE="${MOCK_RALPH_EXIT_CODE:-0}"
DELAY_MS="${MOCK_RALPH_DELAY_MS:-50}"

mkdir -p "$EVENTS_DIR"

EVENTS_FILE="$EVENTS_DIR/events-${LOOP_ID}.jsonl"
TS=$(date -u +"%Y-%m-%dT%H:%M:%S+00:00")

# Write a progress event
echo "{\"ts\":\"${TS}\",\"topic\":\"iteration.start\",\"payload\":\"starting\",\"iteration\":0,\"hat\":\"builder\",\"triggered\":\"loop\"}" >> "$EVENTS_FILE"

# Small delay
DELAY_S=$(awk "BEGIN{printf \"%.3f\", $DELAY_MS/1000}")
sleep "$DELAY_S"

# Write completion promise event if specified
if [ -n "$COMPLETION_PROMISE" ]; then
    TS2=$(date -u +"%Y-%m-%dT%H:%M:%S+00:00")
    echo "{\"ts\":\"${TS2}\",\"topic\":\"${COMPLETION_PROMISE}\",\"payload\":\"done\",\"iteration\":1,\"hat\":\"builder\",\"triggered\":\"loop\"}" >> "$EVENTS_FILE"
fi

exit "$EXIT_CODE"
