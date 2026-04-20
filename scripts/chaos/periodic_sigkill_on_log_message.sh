#!/bin/bash

set -e

SERVICE="rollup"
READY_URL="http://127.0.0.1:12346/sequencer/ready"
READY_TIMEOUT=120
LOG_FILE="/my/proccess.log"
LOG_PATTERN="Commiting a group"
LOG_WAIT_TIMEOUT=300
DELAY_MIN_MS=0
DELAY_MAX_MS=0

usage() {
    echo "Usage: $0 [-d|--delay MIN_MS MAX_MS] [-l|--log FILE] [-p|--pattern PATTERN]"
    echo "  -d, --delay MIN MAX    Sleep random ms (MIN-MAX) after log pattern before kill"
    echo "  -l, --log FILE         Log file to watch (default: $LOG_FILE)"
    echo "  -p, --pattern PATTERN  Log pattern to match (default: $LOG_PATTERN)"
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
        *) echo "Unknown option: $1"; usage ;;
    esac
done

kill_on_log_pattern() {
    local pid
    pid=$(systemctl show -p MainPID --value "$SERVICE")
    if [[ "$pid" -le 0 ]]; then
        echo "$(date): Service not running"
        return 1
    fi

    echo "$(date): Waiting for pattern: '$LOG_PATTERN' (timeout: ${LOG_WAIT_TIMEOUT}s)"

    if ! timeout "$LOG_WAIT_TIMEOUT" bash -c "tail -n 0 -f '$LOG_FILE' | grep -m 1 -q '$LOG_PATTERN'"; then
        echo "$(date): Timeout waiting for log pattern" >&2
        exit 1
    fi

    echo "$(date): Pattern matched"

    if [[ "$DELAY_MAX_MS" -gt 0 ]]; then
        delay_ms=$((RANDOM % (DELAY_MAX_MS - DELAY_MIN_MS + 1) + DELAY_MIN_MS))
        delay_sec=$(awk "BEGIN {printf \"%.3f\", $delay_ms / 1000}")
        echo "$(date): Delaying ${delay_ms}ms"
        sleep "$delay_sec"
    fi

    # Re-fetch PID in case it changed
    pid=$(systemctl show -p MainPID --value "$SERVICE")
    if [[ "$pid" -gt 0 ]]; then
        echo "$(date): Killing $SERVICE (PID $pid) with SIGKILL"
        kill -9 "$pid" 2>/dev/null || true
        sleep 1
    fi
}

while true; do
    kill_on_log_pattern

    sleep_sec=$((RANDOM % 5 + 3))
    echo "$(date): Sleeping $sleep_sec seconds"
    sleep "$sleep_sec"

    echo "$(date): Starting $SERVICE"
    systemctl start "$SERVICE"

    echo "$(date): Waiting for ready (timeout: ${READY_TIMEOUT}s)"
    start_time=$(date +%s)
    while true; do
        if curl -s -o /dev/null -w '' --max-time 5 "$READY_URL"; then
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