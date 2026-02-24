#!/usr/bin/env bash
set -euo pipefail

log() {
  printf '[hive-rpc] %s\n' "$*"
}

usage() {
  cat <<'EOF'
Usage: run-rpc-compat.sh [options]

Runs Hive ethereum/rpc-compat for sov-demo-rollup, stores timestamped logs,
and prints a compact summary.

Options:
  --build-image            Build sov-demo-rollup-hive image before running Hive
  --tag <suffix>           Result directory suffix (default: run)
  --profile <name>         Built-in profile: full|p0|p0-nonhistorical (default: full)
  --sim <name>             Hive simulator (default: ethereum/rpc-compat)
  --client <name>          Hive client (default: sov-demo-rollup)
  --limit <regex>          Optional Hive --sim.limit regex (overrides --profile)
  --check-timeout <dur>    Hive --client.checktimelimit (default: 10m)
  --hive-dir <path>        Hive repo path (default: $HOME/workspace/hive)
  --results-base <path>    Base log dir (default: <hive-dir>/workspace/logs)
  --image-tag <tag>        Docker tag for sov-demo-rollup-hive (default: local)
  --chain-id <id>          Build arg HIVE_CHAIN_ID (default: 3503995874084926)
  --exit-on-fail           Exit non-zero if Hive reports failing tests
  -h, --help               Show this help

Environment overrides:
  HIVE_LOGLEVEL            default 2
  HIVE_SIM_LOGLEVEL        default 2
EOF
}

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SDK_ROOT="$(cd "${SCRIPT_DIR}/../../.." && pwd)"
P0_SCOPE_FILE="${SCRIPT_DIR}/p0-nonhistorical-tests.regex"

BUILD_IMAGE=0
RUN_TAG="run"
PROFILE="full"
SIM="ethereum/rpc-compat"
CLIENT="sov-demo-rollup"
SIM_LIMIT=""
PROFILE_SCOPE_REGEX=""
CHECK_TIMEOUT="10m"
HIVE_DIR="${HOME}/workspace/hive"
RESULTS_BASE=""
IMAGE_TAG="local"
CHAIN_ID="3503995874084926"
EXIT_ON_FAIL=0

while (($# > 0)); do
  case "$1" in
    --build-image)
      BUILD_IMAGE=1
      shift
      ;;
    --tag)
      RUN_TAG="${2:?missing value for --tag}"
      shift 2
      ;;
    --profile)
      PROFILE="${2:?missing value for --profile}"
      shift 2
      ;;
    --sim)
      SIM="${2:?missing value for --sim}"
      shift 2
      ;;
    --client)
      CLIENT="${2:?missing value for --client}"
      shift 2
      ;;
    --limit)
      SIM_LIMIT="${2:?missing value for --limit}"
      shift 2
      ;;
    --check-timeout)
      CHECK_TIMEOUT="${2:?missing value for --check-timeout}"
      shift 2
      ;;
    --hive-dir)
      HIVE_DIR="${2:?missing value for --hive-dir}"
      shift 2
      ;;
    --results-base)
      RESULTS_BASE="${2:?missing value for --results-base}"
      shift 2
      ;;
    --image-tag)
      IMAGE_TAG="${2:?missing value for --image-tag}"
      shift 2
      ;;
    --chain-id)
      CHAIN_ID="${2:?missing value for --chain-id}"
      shift 2
      ;;
    --exit-on-fail)
      EXIT_ON_FAIL=1
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      log "Unknown argument: $1"
      usage
      exit 1
      ;;
  esac
done

if [[ -z "${SIM_LIMIT}" ]]; then
  case "${PROFILE}" in
    full)
      ;;
    p0|p0-nonhistorical)
      # P0 non-historical smoke surface: basic network/transaction correctness,
      # intentionally excluding fixture-history dependent reads.
      # rpc-compat's simulator-level filtering is too coarse for method subsets,
      # so this profile runs the full suite and evaluates pass/fail only for the
      # scoped P0 test names listed in ${P0_SCOPE_FILE}.
      if [[ ! -f "${P0_SCOPE_FILE}" ]]; then
        log "Missing P0 scope file: ${P0_SCOPE_FILE}"
        exit 1
      fi
      P0_SCOPE_TERMS="$(
        awk '
          /^[[:space:]]*(#|$)/ { next }
          { out = (out == "" ? $0 : out "|" $0) }
          END { print out }
        ' "${P0_SCOPE_FILE}"
      )"
      if [[ -z "${P0_SCOPE_TERMS}" ]]; then
        log "P0 scope file is empty: ${P0_SCOPE_FILE}"
        exit 1
      fi
      PROFILE_SCOPE_REGEX="^(${P0_SCOPE_TERMS})$"
      ;;
    *)
      log "Unknown profile: ${PROFILE} (expected full|p0|p0-nonhistorical)"
      exit 1
      ;;
  esac
fi

if [[ ! -d "${HIVE_DIR}" ]]; then
  log "Hive directory not found: ${HIVE_DIR}"
  exit 1
fi

if [[ -z "${RESULTS_BASE}" ]]; then
  RESULTS_BASE="${HIVE_DIR}/workspace/logs"
fi

TIMESTAMP="$(date +%Y%m%d-%H%M%S)"
RUN_DIR="${RESULTS_BASE}/full-${TIMESTAMP}-${RUN_TAG}"
RUN_LOG="${RUN_DIR}/runner.log"
HIVE_LOGLEVEL="${HIVE_LOGLEVEL:-2}"
HIVE_SIM_LOGLEVEL="${HIVE_SIM_LOGLEVEL:-2}"

mkdir -p "${RUN_DIR}"

if [[ "${BUILD_IMAGE}" == "1" ]]; then
  log "Building image sov-demo-rollup-hive:${IMAGE_TAG} (chain_id=${CHAIN_ID})"
  docker build \
    --build-arg "HIVE_CHAIN_ID=${CHAIN_ID}" \
    -f "${SCRIPT_DIR}/Dockerfile" \
    -t "sov-demo-rollup-hive:${IMAGE_TAG}" \
    "${SDK_ROOT}"
fi

HIVE_CMD=(
  ./hive
  --sim "${SIM}"
  --client "${CLIENT}"
  --results-root "${RUN_DIR}"
  --loglevel "${HIVE_LOGLEVEL}"
  --sim.loglevel "${HIVE_SIM_LOGLEVEL}"
  --client.checktimelimit "${CHECK_TIMEOUT}"
)
if [[ -n "${SIM_LIMIT}" ]]; then
  HIVE_CMD+=(--sim.limit "${SIM_LIMIT}")
fi

log "Run dir: ${RUN_DIR}"
log "Profile: ${PROFILE}"
if [[ -n "${SIM_LIMIT}" ]]; then
  log "Sim limit regex: ${SIM_LIMIT}"
fi
if [[ -n "${PROFILE_SCOPE_REGEX}" ]]; then
  log "Profile scope regex: ${PROFILE_SCOPE_REGEX}"
fi
log "Running: ${HIVE_CMD[*]}"

set +e
(
  cd "${HIVE_DIR}"
  "${HIVE_CMD[@]}" 2>&1 | tee "${RUN_LOG}"
)
HIVE_RC=$?
set -e

RESULT_JSON="$(find "${RUN_DIR}" -maxdepth 1 -type f -name '*.json' ! -name 'hive.json' | head -n1 || true)"
if [[ -z "${RESULT_JSON}" ]]; then
  log "No result json found in ${RUN_DIR}"
  log "Hive exit code: ${HIVE_RC}"
  [[ "${EXIT_ON_FAIL}" == "1" ]] && exit "${HIVE_RC}"
  exit 0
fi

SUMMARY="$(
  jq -r '
    .testCases
    | to_entries
    | map(.value.summaryResult.pass)
    | "total=\(length) pass=\(map(select(.))|length) fail=\(map(select(.|not))|length)"
  ' "${RESULT_JSON}"
)"

log "Summary: ${SUMMARY}"
log "Result json: ${RESULT_JSON}"
log "Runner log: ${RUN_LOG}"

log "Top failing method buckets:"
jq -r '
  .testCases
  | to_entries[]
  | select(.value.summaryResult.pass|not)
  | .value.name
' "${RESULT_JSON}" \
  | sed 's|/.*||' \
  | sort \
  | uniq -c \
  | sort -nr \
  | head -n 20 \
  | sed 's/^/[hive-rpc]   /'

PROFILE_FAIL_COUNT=""
if [[ -n "${PROFILE_SCOPE_REGEX}" ]]; then
  PROFILE_SUMMARY="$(
    jq -r --arg re "${PROFILE_SCOPE_REGEX}" '
      .testCases
      | to_entries
      | map(select(.value.name | test($re)))
      | "total=\(length) pass=\(map(select(.value.summaryResult.pass))|length) fail=\(map(select(.value.summaryResult.pass|not))|length)"
    ' "${RESULT_JSON}"
  )"
  PROFILE_FAIL_COUNT="$(
    jq -r --arg re "${PROFILE_SCOPE_REGEX}" '
      .testCases
      | to_entries
      | map(select(.value.name | test($re)))
      | map(select(.value.summaryResult.pass|not))
      | length
    ' "${RESULT_JSON}"
  )"
  log "Profile summary: ${PROFILE_SUMMARY}"

  if [[ "${PROFILE_FAIL_COUNT}" != "0" ]]; then
    log "Profile failing tests:"
    jq -r --arg re "${PROFILE_SCOPE_REGEX}" '
      .testCases
      | to_entries[]
      | select(.value.name | test($re))
      | select(.value.summaryResult.pass|not)
      | .value.name
    ' "${RESULT_JSON}" \
      | head -n 40 \
      | sed 's/^/[hive-rpc]   /'
  fi
fi

if [[ "${EXIT_ON_FAIL}" == "1" ]]; then
  if [[ -n "${PROFILE_SCOPE_REGEX}" ]]; then
    if [[ "${PROFILE_FAIL_COUNT}" != "0" ]]; then
      exit 1
    fi
    exit 0
  fi
  exit "${HIVE_RC}"
fi
