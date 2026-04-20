#!/usr/bin/env bash
# toxi_scenario.sh — toxic + scenario CLI for the sovereign-soak toxiproxy setup.
#
# Targets:
#   primary   → *_1 proxies   (default)
#   replica   → *_2 proxies
#   both      → *_1 and *_2
#   tertiary  → postgres_3    (postgres class only — no celestia _3 proxies exist)
#   all       → every proxy   (clear only)
#
# Usage:
#   toxi_scenario.sh list
#   toxi_scenario.sh clear [primary|replica|both|tertiary|all]   # default: all
#   toxi_scenario.sh toxic <NAME> [primary|replica|both|tertiary]  # default: primary
#   toxi_scenario.sh scenario <ID> [primary|replica|both|tertiary] # default: primary
#
# tertiary is only valid for pg-* toxics and pg-only scenarios (P1/P3/P4).
# Reaching for tertiary on an rpc/grpc class is a hard error.
#
# Named toxics:
#   rpc-latency    celestia_rpc  latency 300ms ±200ms
#   rpc-timeout    celestia_rpc  timeout 40000ms
#   pg-reset       postgres      reset_peer 0ms
#   pg-latency     postgres      latency 50ms ±20ms
#
# Scenarios (P1..P7 single-rollup; R1..R2 replica-only):
#   P1  postgres      reset_peer 0ms,         toxicity 0.10
#   P2  celestia_rpc  timeout 8000ms,         toxicity 1.00
#   P3  postgres      timeout 15000ms,        toxicity 1.00
#   P4  postgres      latency 100ms ±25ms,    toxicity 1.00
#   P5  celestia_rpc  latency 6000ms ±1000ms, toxicity 1.00
#   P6  celestia_rpc  latency 6000ms ±1000ms (1.00) + postgres reset_peer 0ms (0.10)
#   P7  celestia_grpc reset_peer 0ms,         toxicity 0.50
#   R1  postgres_2    latency 8000ms ±500ms,  toxicity 1.00   (target: replica)
#   R2  postgres_2    reset_peer 0ms,         toxicity 0.20   (target: replica)
#
# Apply semantics: `toxic` and `scenario` clear the proxies they touch (for the chosen
# target) before applying, so calling the same scenario twice never stacks toxics.

set -euo pipefail

CLI="${TOXIPROXY_CLI:-toxiproxy-cli}"
HOST="${TOXIPROXY_HOST:-127.0.0.1:8474}"
export TOXIPROXY_URL="http://${HOST}"

die() { echo "error: $*" >&2; exit 2; }

for bin in "${CLI}" curl jq; do
  command -v "${bin}" >/dev/null || die "${bin} not found in PATH"
done

# ----- proxy name expansion --------------------------------------------------

# class_proxies <class> <target>   → prints proxy names, one per line
class_proxies() {
  local class="$1" target="$2"
  case "${target}" in
    primary) printf '%s_1\n' "${class}" ;;
    replica) printf '%s_2\n' "${class}" ;;
    both)    printf '%s_1\n%s_2\n' "${class}" "${class}" ;;
    tertiary)
      [[ "${class}" == "postgres" ]] \
        || die "tertiary target only valid for postgres class (got '${class}')"
      printf 'postgres_3\n'
      ;;
    *) die "bad target '${target}' (expected primary|replica|both|tertiary)" ;;
  esac
}

# all_proxies_for_target <target>   → every proxy for the target (used by `clear`)
all_proxies_for_target() {
  local target="$1"
  case "${target}" in
    primary)  echo "postgres_1 celestia_rpc_1 celestia_grpc_1" ;;
    replica)  echo "postgres_2 celestia_rpc_2 celestia_grpc_2" ;;
    both)     echo "postgres_1 celestia_rpc_1 celestia_grpc_1 postgres_2 celestia_rpc_2 celestia_grpc_2" ;;
    tertiary) echo "postgres_3" ;;
    all)      echo "postgres_1 postgres_2 postgres_3 celestia_rpc_1 celestia_rpc_2 celestia_grpc_1 celestia_grpc_2" ;;
    *) die "bad target '${target}' (expected primary|replica|both|tertiary|all)" ;;
  esac
}

# ----- primitives ------------------------------------------------------------

clear_proxy() {
  local proxy="$1"
  # Delete every toxic on this proxy. Admin API returns {name,type,...} per toxic.
  local names
  names="$(curl -fsS "${TOXIPROXY_URL}/proxies/${proxy}/toxics" | jq -r '.[].name')" || return 0
  local t
  for t in ${names}; do
    curl -fsS -X DELETE "${TOXIPROXY_URL}/proxies/${proxy}/toxics/${t}" >/dev/null
  done
}

# add_toxic <proxy> <toxic_name> <type> <toxicity> [attr=value ...]
add_toxic() {
  local proxy="$1" name="$2" type="$3" toxicity="$4"; shift 4
  local args=(--type "${type}" --toxicName "${name}" --toxicity "${toxicity}")
  local kv
  for kv in "$@"; do args+=(--attribute "${kv}"); done
  "${CLI}" toxic add "${args[@]}" "${proxy}"
}

# ----- named toxics (applied to all proxies of the given class+target) ------

apply_named_toxic() {
  local name="$1" target="${2:-primary}"
  local class
  case "${name}" in
    rpc-latency|rpc-timeout) class="celestia_rpc" ;;
    pg-reset|pg-latency)     class="postgres" ;;
    *) die "unknown toxic '${name}' (expected rpc-latency|rpc-timeout|pg-reset|pg-latency)" ;;
  esac

  local proxies; proxies="$(class_proxies "${class}" "${target}")"
  local p
  for p in ${proxies}; do clear_proxy "${p}"; done

  for p in ${proxies}; do
    case "${name}" in
      rpc-latency) add_toxic "${p}" "${name}" latency    1.0 latency=300  jitter=200 ;;
      rpc-timeout) add_toxic "${p}" "${name}" timeout    0.2 timeout=40000 ;;
      pg-reset)    add_toxic "${p}" "${name}" reset_peer 0.1 timeout=0 ;;
      pg-latency)  add_toxic "${p}" "${name}" latency    1.0 latency=50   jitter=20 ;;
    esac
  done
}

# ----- scenarios -------------------------------------------------------------

scenario_P1() { # postgres reset_peer 0ms toxicity 0.10
  local target="$1" p
  for p in $(class_proxies postgres "${target}"); do
    clear_proxy "${p}"
    add_toxic "${p}" "P1" reset_peer 0.10 timeout=0
  done
}

scenario_P2() { # celestia_rpc timeout 8000ms toxicity 0.15
  local target="$1" p
  for p in $(class_proxies celestia_rpc "${target}"); do
    clear_proxy "${p}"
    add_toxic "${p}" "P2" timeout 0.15 timeout=8000
  done
}

scenario_P3() { # postgres timeout 15000ms toxicity 1.00
  local target="$1" p
  for p in $(class_proxies postgres "${target}"); do
    clear_proxy "${p}"
    add_toxic "${p}" "P3" timeout 1.0 timeout=15000
  done
}

scenario_P4() { # postgres latency 100ms ±25ms toxicity 1.00
  local target="$1" p
  for p in $(class_proxies postgres "${target}"); do
    clear_proxy "${p}"
    add_toxic "${p}" "P4" latency 1.0 latency=100 jitter=25
  done
}

scenario_P5() { # celestia_rpc latency 6000ms ±1000ms toxicity 1.00
  local target="$1" p
  for p in $(class_proxies celestia_rpc "${target}"); do
    clear_proxy "${p}"
    add_toxic "${p}" "P5" latency 0.4 latency=4000 jitter=1000
  done
}

scenario_P6() { # P5 rpc latency + P1 pg reset
  local target="$1" p
  for p in $(class_proxies celestia_rpc "${target}"); do
    clear_proxy "${p}"
    add_toxic "${p}" "P6_rpc_latency" latency 1.0 latency=6000 jitter=1000
  done
  for p in $(class_proxies postgres "${target}"); do
    clear_proxy "${p}"
    add_toxic "${p}" "P6_pg_reset" reset_peer 0.10 timeout=0
  done
}

scenario_P7() { # celestia_grpc reset_peer 0ms toxicity 0.50
  local target="$1" p
  for p in $(class_proxies celestia_grpc "${target}"); do
    clear_proxy "${p}"
    add_toxic "${p}" "P7" reset_peer 0.50 timeout=0
  done
}

scenario_R1() { # replica postgres latency 8000ms ±500ms toxicity 1.00
  clear_proxy postgres_2
  add_toxic postgres_2 "R1" latency 1.0 latency=8000 jitter=500
}

scenario_R2() { # replica postgres reset_peer 0ms toxicity 0.20
  clear_proxy postgres_2
  add_toxic postgres_2 "R2" reset_peer 0.20 timeout=0
}

# ----- dispatch --------------------------------------------------------------

cmd_list() {
  "${CLI}" list
  echo
  local p
  for p in postgres_1 postgres_2 postgres_3 celestia_rpc_1 celestia_rpc_2 celestia_grpc_1 celestia_grpc_2; do
    echo "---- ${p} ----"
    "${CLI}" inspect "${p}" || true
  done
}

cmd_clear() {
  local target="${1:-all}"
  local p
  for p in $(all_proxies_for_target "${target}"); do
    clear_proxy "${p}"
  done
  echo "cleared toxics on target: ${target}"
}

cmd_toxic() {
  [[ $# -ge 1 ]] || die "usage: toxi_scenario.sh toxic <NAME> [primary|replica|both|tertiary]"
  apply_named_toxic "$1" "${2:-primary}"
  echo "applied toxic '$1' on target: ${2:-primary}"
}

cmd_scenario() {
  [[ $# -ge 1 ]] || die "usage: toxi_scenario.sh scenario <ID> [primary|replica|both|tertiary]"
  local id="$1" target="${2:-primary}"
  case "${id}" in
    P2|P5|P6|P7)
      [[ "${target}" != "tertiary" ]] \
        || die "scenario ${id} touches a celestia class — tertiary is postgres-only"
      "scenario_${id}" "${target}"
      ;;
    P1|P3|P4) "scenario_${id}" "${target}" ;;
    R1|R2)
      [[ -z "${2:-}" || "${2}" == "replica" ]] \
        || die "scenario ${id} is replica-only; do not pass '${2}'"
      "scenario_${id}"
      ;;
    *) die "unknown scenario '${id}' (expected P1..P7 or R1..R2)" ;;
  esac
  echo "applied scenario ${id} on target: ${target}"
}

main() {
  [[ $# -ge 1 ]] || { awk 'NR>1 && /^[^#]/{exit} NR>1{print}' "$0"; exit 1; }
  local cmd="$1"; shift
  case "${cmd}" in
    list)     cmd_list "$@" ;;
    clear)    cmd_clear "$@" ;;
    toxic)    cmd_toxic "$@" ;;
    scenario) cmd_scenario "$@" ;;
    -h|--help|help) awk 'NR>1 && /^[^#]/{exit} NR>1{print}' "$0" ;;
    *) die "unknown command '${cmd}'" ;;
  esac
}

main "$@"