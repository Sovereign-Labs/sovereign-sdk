#!/usr/bin/env bash
# periodic_sigkill_on_log_message.sh — SIGKILL the rollup process when a log line matches.
#
# Loop: tail -f LOG_FILE, grep for PATTERN, optional random delay, KILL_CMD,
# wait for restart via START_CMD + READY_URL, sleep, repeat.
#
# A log file path is REQUIRED (-l). For docker, redirect compose logs into a file:
#   docker compose logs -f rollup > /tmp/rollup.log &
#   ./periodic_sigkill_on_log_message.sh -l /tmp/rollup.log -p "Commiting a group"
#
# Env (all overridable; defaults are baremetal+systemd):
#   KILL_CMD       SIGKILL                default: bash -c 'pid=$(systemctl show -p MainPID --value rollup);
#                                                          [ "$pid" -gt 0 ] && kill -9 "$pid" || true'
#   START_CMD      start                  default: systemctl start rollup
#   READY_URL      readiness probe        default: http://127.0.0.1:12346/sequencer/ready
#   READY_TIMEOUT  seconds                default: 120
#   LOG_WAIT_TIMEOUT seconds              default: 300

set -euo pipefail

# shellcheck disable=SC2016  # the inner $pid expansion happens in the bash -c subshell, not here
KILL_CMD="${KILL_CMD:-bash -c 'pid=\$(systemctl show -p MainPID --value rollup); [ \"\$pid\" -gt 0 ] && kill -9 \"\$pid\" || true'}"
START_CMD="${START_CMD:-systemctl start rollup}"
READY_URL="${READY_URL:-http://127.0.0.1:12346/sequencer/ready}"
READY_TIMEOUT="${READY_TIMEOUT:-120}"
LOG_WAIT_TIMEOUT="${LOG_WAIT_TIMEOUT:-300}"

LOG_FILE=""
LOG_PATTERN="Commiting a group"
DELAY_MIN_MS=0
DELAY_MAX_MS=0

usage() {
    cat <<EOF >&2
Usage: $0 -l|--log FILE [-p|--pattern PATTERN] [-d|--delay MIN_MS MAX_MS]
  -l, --log FILE         Log file to watch (REQUIRED)
  -p, --pattern PATTERN  Log pattern to match (default: $LOG_PATTERN)
  -d, --delay MIN MAX    Sleep random ms in [MIN,MAX] after match before kill

Override restart behavior via KILL_CMD / START_CMD / READY_URL env vars.
EOF
    exit 1
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        -d|--delay)
            DELAY_MIN_MS="$2"
            DELAY_MAX_MS="$3"
            shift 3
            ;;
        -l|--log)
            LOG_FILE="$2"
            shift 2
            ;;
        -p|--pattern)
            LOG_PATTERN="$2"
            shift 2
            ;;
        -h|--help) usage ;;
        *) echo "Unknown option: $1" >&2; usage ;;
    esac
done

[ -n "$LOG_FILE" ] || { echo "error: --log is required" >&2; usage; }
[ -f "$LOG_FILE" ] || { echo "error: log file not found: $LOG_FILE" >&2; exit 1; }

command -v curl >/dev/null || { echo "curl required" >&2; exit 1; }

kill_on_log_pattern() {
    echo "$(date): Waiting for pattern: '$LOG_PATTERN' (timeout: ${LOG_WAIT_TIMEOUT}s)"

    if ! timeout "$LOG_WAIT_TIMEOUT" bash -c "tail -n 0 -f '$LOG_FILE' | grep -m 1 -q '$LOG_PATTERN'"; then
        echo "$(date): Timeout waiting for log pattern" >&2
        exit 1
    fi

    echo "$(date): Pattern matched"

    if [[ "$DELAY_MAX_MS" -gt 0 ]]; then
        delay_ms=$((RANDOM % (DELAY_MAX_MS - DELAY_MIN_MS + 1) + DELAY_MIN_MS))
        # `sleep` accepts decimals in coreutils; fractional ms is overkill here.
        delay_sec="$((delay_ms / 1000)).$(printf '%03d' "$((delay_ms % 1000))")"
        echo "$(date): Delaying ${delay_ms}ms"
        sleep "$delay_sec"
    fi

    echo "$(date): Killing rollup (SIGKILL)"
    eval "$KILL_CMD" || true
    sleep 1
}

while true; do
    kill_on_log_pattern

    sleep_sec=$((RANDOM % 5 + 3))
    echo "$(date): Sleeping $sleep_sec seconds"
    sleep "$sleep_sec"

    echo "$(date): Starting rollup"
    eval "$START_CMD"

    echo "$(date): Waiting for ready (timeout: ${READY_TIMEOUT}s)"
    start_time=$(date +%s)
    while true; do
        if curl --fail -s -o /dev/null --max-time 5 "$READY_URL"; then
            echo "$(date): Ready"
            break
        fi
        if (( $(date +%s) - start_time >= READY_TIMEOUT )); then
            echo "$(date): Timeout waiting for ready" >&2
            exit 1
        fi
        sleep 1
    done

    sleep_min=$((RANDOM % 6 + 5))
    echo "$(date): Sleeping $sleep_min minutes"
    sleep "${sleep_min}m"
done
