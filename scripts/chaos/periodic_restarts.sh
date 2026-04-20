#!/bin/bash

set -e

SERVICE="rollup"
READY_URL="http://127.0.0.1:12346/sequencer/ready"
READY_TIMEOUT=120
FORCE_KILL=false

usage() {
    echo "Usage: $0 [-f|--force]"
    echo "  -f, --force    Kill service with SIGKILL instead of graceful stop"
    exit 1
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        -f|--force) FORCE_KILL=true; shift ;;
        -h|--help) usage ;;
        *) echo "Unknown option: $1"; usage ;;
    esac
done

stop_service() {
    if $FORCE_KILL; then
        pid=$(systemctl show -p MainPID --value "$SERVICE")
        if [[ "$pid" -gt 0 ]]; then
            echo "$(date): Killing $SERVICE (PID $pid) with SIGKILL"
            kill -9 "$pid" 2>/dev/null || true
            # Wait for systemd to notice
            sleep 1
        else
            echo "$(date): Service not running, nothing to kill"
        fi
    else
        echo "$(date): Stopping $SERVICE gracefully"
        systemctl stop "$SERVICE"
    fi
}

while true; do
    stop_service

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