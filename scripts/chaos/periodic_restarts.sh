#!/usr/bin/env bash
# periodic_restarts.sh — restart the rollup process on a random cadence.
#
# Each loop iteration: stop, sleep 3-7s, start, wait for ready, sleep 5-10min, repeat.
#
# Env (all overridable; defaults are baremetal+systemd):
#   STOP_CMD       graceful stop          default: systemctl stop rollup
#   KILL_CMD       SIGKILL                default: bash -c 'pid=$(systemctl show -p MainPID --value rollup);
#                                                          [ "$pid" -gt 0 ] && kill -9 "$pid" || true'
#   START_CMD      start                  default: systemctl start rollup
#   READY_URL      readiness probe URL    default: http://127.0.0.1:12346/sequencer/ready
#   READY_TIMEOUT  seconds to wait ready  default: 120
#
# Docker example:
#   STOP_CMD="docker compose stop rollup" \
#     KILL_CMD="docker compose kill -s KILL rollup" \
#     START_CMD="docker compose start rollup" \
#     ./periodic_restarts.sh

set -euo pipefail

STOP_CMD="${STOP_CMD:-systemctl stop rollup}"
# shellcheck disable=SC2016  # the inner $pid expansion happens in the bash -c subshell, not here
KILL_CMD="${KILL_CMD:-bash -c 'pid=\$(systemctl show -p MainPID --value rollup); [ \"\$pid\" -gt 0 ] && kill -9 \"\$pid\" || true'}"
START_CMD="${START_CMD:-systemctl start rollup}"
READY_URL="${READY_URL:-http://127.0.0.1:12346/sequencer/ready}"
READY_TIMEOUT="${READY_TIMEOUT:-120}"

FORCE_KILL=false

usage() {
    echo "Usage: $0 [-f|--force]"
    echo "  -f, --force    Kill via KILL_CMD (SIGKILL) instead of STOP_CMD"
    echo
    echo "Override behavior with STOP_CMD / KILL_CMD / START_CMD / READY_URL env vars."
    exit 1
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        -f|--force) FORCE_KILL=true; shift ;;
        -h|--help) usage ;;
        *) echo "Unknown option: $1" >&2; usage ;;
    esac
done

command -v curl >/dev/null || { echo "curl required" >&2; exit 1; }

stop_service() {
    if $FORCE_KILL; then
        echo "$(date): Force-killing service"
        eval "$KILL_CMD" || true
        sleep 1
    else
        echo "$(date): Stopping service gracefully"
        eval "$STOP_CMD"
    fi
}

while true; do
    stop_service

    sleep_sec=$((RANDOM % 5 + 3))
    echo "$(date): Sleeping $sleep_sec seconds"
    sleep "$sleep_sec"

    echo "$(date): Starting service"
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
