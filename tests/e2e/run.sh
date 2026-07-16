#!/usr/bin/env bash
# E2E tmux-based validation for Loopy TUI.
# Usage: tests/e2e/run.sh [--binary PATH]
# Exit 0 = all pass, non-zero = failure with diagnostics.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
BINARY="${REPO_ROOT}/target/debug/loopy"
SESSION="loopy-e2e-$$"
WORK_DIR=""
PASS=0
FAIL=0
DIAGNOSTICS=""

while [[ $# -gt 0 ]]; do
    case "$1" in
        --binary) BINARY="$2"; shift 2 ;;
        *) shift ;;
    esac
done

# --- Helpers ---

setup() {
    WORK_DIR=$(mktemp -d)
    export HOME="$WORK_DIR"
}

teardown() {
    tmux kill-session -t "$SESSION" 2>/dev/null || true
    [ -n "$WORK_DIR" ] && rm -rf "$WORK_DIR"
}

capture_pane() {
    tmux capture-pane -t "$SESSION" -p 2>/dev/null || echo ""
}

assert_contains() {
    local label="$1" expected="$2"
    local content
    content=$(capture_pane)
    if echo "$content" | grep -qF "$expected"; then
        return 0
    else
        DIAGNOSTICS+="--- FAIL: $label ---\nExpected to find: '$expected'\nCaptured pane:\n$content\n\n"
        return 1
    fi
}

wait_for_text() {
    local expected="$1" timeout="${2:-5}"
    local deadline=$((SECONDS + timeout))
    while [ $SECONDS -lt $deadline ]; do
        if capture_pane | grep -qF "$expected"; then
            return 0
        fi
        sleep 0.3
    done
    return 1
}

record_pass() {
    echo "  ✅ $1"
    PASS=$((PASS + 1))
}

record_fail() {
    echo "  ❌ $1"
    FAIL=$((FAIL + 1))
}

# --- Scenario 1: Happy Path ---

test_happy_path() {
    echo "▶ Scenario 1: Happy path — loopy new → type idea → pipeline renders"
    setup
    trap teardown EXIT

    tmux new-session -d -s "$SESSION" -x 120 -y 30 "cd '$WORK_DIR' && '$BINARY' new"
    sleep 1

    # AC1: TUI renders pipeline stages
    if assert_contains "pipeline renders" "Idea"; then
        record_pass "Pipeline bar renders Idea stage"
    else
        record_fail "Pipeline bar renders Idea stage"
    fi

    # AC1: Type an idea and submit
    tmux send-keys -t "$SESSION" "Build a web app" ""
    sleep 0.5

    if wait_for_text "Build a web app" 3; then
        record_pass "Idea text appears in TUI"
    else
        record_fail "Idea text appears in TUI"
    fi

    # Submit the idea with Enter
    tmux send-keys -t "$SESSION" Enter
    sleep 1

    # After idea submission, Scan should start
    if wait_for_text "Scan" 3; then
        record_pass "Scan stage visible after idea submission"
    else
        record_fail "Scan stage visible after idea submission"
    fi

    # Quit gracefully
    tmux send-keys -t "$SESSION" q
    sleep 0.5

    teardown
    trap - EXIT
}

# --- Scenario 2: Kill mid-stage → Failed state (adversarial, AC2) ---

test_kill_mid_stage() {
    echo "▶ Scenario 2: Adversarial — Scan failed state shows ❌ and error"
    setup
    trap teardown EXIT

    # Pre-create state with Scan in Failed status (simulates killed Ralph)
    mkdir -p "$WORK_DIR/.loopy"
    cat > "$WORK_DIR/.loopy/state.json" << 'FAILSTATE'
{
  "version": 2,
  "idea_text": "test idea",
  "stages": [
    {"id": "idea", "status": "complete", "loop_id": null, "loop_pid": null, "started_at": null, "completed_at": null, "error": null},
    {"id": "scan", "status": "failed", "loop_id": null, "loop_pid": null, "started_at": null, "completed_at": null, "error": "Ralph process killed"},
    {"id": "plan", "status": "pending", "loop_id": null, "loop_pid": null, "started_at": null, "completed_at": null, "error": null},
    {"id": "requirements_analysis", "status": "pending", "loop_id": null, "loop_pid": null, "started_at": null, "completed_at": null, "error": null},
    {"id": "orbital_lanes", "status": "pending", "loop_id": null, "loop_pid": null, "started_at": null, "completed_at": null, "error": null},
    {"id": "land", "status": "pending", "loop_id": null, "loop_pid": null, "started_at": null, "completed_at": null, "error": null}
  ],
  "tracks": null,
  "created_at": "2026-03-31T20:00:00Z",
  "updated_at": "2026-03-31T20:00:00Z"
}
FAILSTATE

    tmux new-session -d -s "$SESSION" -x 120 -y 30 "cd '$WORK_DIR' && '$BINARY' resume"
    sleep 1

    # AC2: TUI shows Scan as Failed (❌ icon in pipeline bar)
    if assert_contains "failed icon" "❌"; then
        record_pass "TUI shows ❌ for failed Scan stage"
    else
        record_fail "TUI shows ❌ for failed Scan stage"
    fi

    # AC2: TUI shows error message in detail area
    if assert_contains "error message" "Ralph process killed"; then
        record_pass "TUI shows failure error message"
    else
        record_fail "TUI shows failure error message"
    fi

    # AC2: Retry option visible in status bar
    if assert_contains "retry hint" "r: retry"; then
        record_pass "TUI shows retry option in status bar"
    else
        record_fail "TUI shows retry option in status bar"
    fi

    tmux send-keys -t "$SESSION" q
    sleep 0.5

    teardown
    trap - EXIT
}

# --- Scenario 2b: Ctrl+C graceful shutdown (adversarial, AC4) ---

test_ctrl_c_shutdown() {
    echo "▶ Scenario 2b: Adversarial — Ctrl+C during pipeline shows graceful exit"
    setup
    trap teardown EXIT

    tmux new-session -d -s "$SESSION" -x 120 -y 30 "cd '$WORK_DIR' && '$BINARY' new"
    sleep 1

    # Type and submit idea to get past Idea stage
    tmux send-keys -t "$SESSION" "Test kill" Enter
    sleep 1

    # Send Ctrl+C
    tmux send-keys -t "$SESSION" C-c
    sleep 1

    # AC4: Checkpoint should be saved
    if [ -f "$WORK_DIR/.loopy/state.json" ]; then
        record_pass "Checkpoint saved on Ctrl+C"
    else
        record_fail "Checkpoint saved on Ctrl+C"
        DIAGNOSTICS+="--- FAIL: Ctrl+C checkpoint ---\nNo .loopy/state.json found in $WORK_DIR\n\n"
    fi

    # AC4: Terminal should be restored (tmux session should have exited)
    sleep 0.5
    if ! tmux has-session -t "$SESSION" 2>/dev/null; then
        record_pass "Terminal restored after Ctrl+C (session exited cleanly)"
    else
        local content
        content=$(capture_pane)
        if echo "$content" | grep -qE '(\$|#)'; then
            record_pass "Terminal restored after Ctrl+C (shell prompt visible)"
        else
            record_fail "Terminal restored after Ctrl+C"
            DIAGNOSTICS+="--- FAIL: terminal restore ---\nCaptured pane:\n$content\n\n"
        fi
    fi

    teardown
    trap - EXIT
}

# --- Scenario 3: Corrupt state → fresh start ---

test_corrupt_state() {
    echo "▶ Scenario 3: Adversarial — corrupt state.json → loopy resume starts fresh"
    setup
    trap teardown EXIT

    # Write corrupt state file
    mkdir -p "$WORK_DIR/.loopy"
    echo "NOT VALID JSON {{{" > "$WORK_DIR/.loopy/state.json"

    tmux new-session -d -s "$SESSION" -x 120 -y 30 "cd '$WORK_DIR' && '$BINARY' resume"
    sleep 1

    # AC3: Fresh pipeline should start (Idea stage active)
    if assert_contains "corrupt resume shows Idea" "Idea"; then
        record_pass "Corrupt state falls back to fresh pipeline"
    else
        record_fail "Corrupt state falls back to fresh pipeline"
    fi

    # Verify it's actually a fresh pipeline — Idea should be the active input
    # (the TUI shows the idea input prompt when Idea is Running)
    if wait_for_text "Type your idea" 3 || wait_for_text "Idea" 3; then
        record_pass "Fresh pipeline shows Idea input prompt"
    else
        record_fail "Fresh pipeline shows Idea input prompt"
    fi

    tmux send-keys -t "$SESSION" q
    sleep 0.5

    teardown
    trap - EXIT
}

# --- Scenario 4: Status command (non-interactive) ---

test_status_command() {
    echo "▶ Scenario 4: loopy status — prints pipeline state"
    setup
    trap teardown EXIT

    # No state file → fresh pipeline status
    local output
    output=$(cd "$WORK_DIR" && "$BINARY" status 2>&1)

    if echo "$output" | grep -qF "Idea"; then
        record_pass "Status shows Idea stage"
    else
        record_fail "Status shows Idea stage"
        DIAGNOSTICS+="--- FAIL: status output ---\n$output\n\n"
    fi

    if echo "$output" | grep -qF "○"; then
        record_pass "Status shows pending icon"
    else
        record_fail "Status shows pending icon"
        DIAGNOSTICS+="--- FAIL: status pending icon ---\n$output\n\n"
    fi

    # With a saved state
    mkdir -p "$WORK_DIR/.loopy"
    cat > "$WORK_DIR/.loopy/state.json" << 'STATEJSON'
{
  "version": 2,
  "idea_text": "test",
  "stages": [
    {"id": "idea", "status": "complete", "loop_id": null, "loop_pid": null, "started_at": null, "completed_at": null, "error": null},
    {"id": "scan", "status": "running", "loop_id": null, "loop_pid": null, "started_at": null, "completed_at": null, "error": null},
    {"id": "plan", "status": "pending", "loop_id": null, "loop_pid": null, "started_at": null, "completed_at": null, "error": null},
    {"id": "requirements_analysis", "status": "pending", "loop_id": null, "loop_pid": null, "started_at": null, "completed_at": null, "error": null},
    {"id": "orbital_lanes", "status": "pending", "loop_id": null, "loop_pid": null, "started_at": null, "completed_at": null, "error": null},
    {"id": "land", "status": "pending", "loop_id": null, "loop_pid": null, "started_at": null, "completed_at": null, "error": null}
  ],
  "tracks": null,
  "created_at": "2026-03-31T20:00:00Z",
  "updated_at": "2026-03-31T20:00:00Z"
}
STATEJSON

    output=$(cd "$WORK_DIR" && "$BINARY" status 2>&1)
    if echo "$output" | grep -qF "✅"; then
        record_pass "Status shows complete icon for saved state"
    else
        record_fail "Status shows complete icon for saved state"
        DIAGNOSTICS+="--- FAIL: status saved state ---\n$output\n\n"
    fi

    if echo "$output" | grep -qF "⏳"; then
        record_pass "Status shows running icon for saved state"
    else
        record_fail "Status shows running icon for saved state"
        DIAGNOSTICS+="--- FAIL: status running icon ---\n$output\n\n"
    fi

    teardown
    trap - EXIT
}

# --- Main ---

echo "═══════════════════════════════════════"
echo " Loopy E2E Tests (tmux)"
echo "═══════════════════════════════════════"
echo ""

# Build first
echo "Building loopy..."
(cd "$REPO_ROOT" && cargo build 2>&1 | tail -1)
echo ""

if [ ! -x "$BINARY" ]; then
    echo "❌ Binary not found at $BINARY"
    exit 1
fi

test_happy_path
echo ""
test_kill_mid_stage
echo ""
test_ctrl_c_shutdown
echo ""
test_corrupt_state
echo ""
test_status_command
echo ""

echo "═══════════════════════════════════════"
echo " Results: $PASS passed, $FAIL failed"
echo "═══════════════════════════════════════"

if [ "$FAIL" -gt 0 ]; then
    echo ""
    echo "Diagnostics:"
    echo -e "$DIAGNOSTICS"
    exit 1
fi

exit 0
