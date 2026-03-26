#!/usr/bin/env bash
# rust-compile-times/scripts/audit.sh
#
# Runs a layered compile-time audit and prints a structured summary.
# Claude reads this output to diagnose bottlenecks without having to
# manually issue each command.
#
# Usage:
#   bash audit.sh                  # debug build, whole workspace
#   bash audit.sh --release        # release build
#   bash audit.sh -p <crate>       # single crate
#   bash audit.sh --no-clean       # skip cargo clean (faster, less accurate)
#
# Requirements: cargo (stable), python3, cargo-llvm-lines (optional)

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# ── Parse arguments ──────────────────────────────────────────────────────────
RELEASE_ARGS=()
PACKAGE_ARGS=()
WORKSPACE_ARGS=(--workspace)
SKIP_CLEAN=0
PROFILE_DISPLAY="debug"
TARGET_DISPLAY="workspace"

while [[ $# -gt 0 ]]; do
  case $1 in
    --release)   RELEASE_ARGS=(--release); PROFILE_DISPLAY="release"; shift ;;
    -p)          PACKAGE_ARGS=(-p "$2"); WORKSPACE_ARGS=(); TARGET_DISPLAY="$2"; shift 2 ;;
    --no-clean)  SKIP_CLEAN=1; shift ;;
    *) echo "Unknown flag: $1" >&2; exit 1 ;;
  esac
done

# ── Temp file cleanup ────────────────────────────────────────────────────────
BUILD_LOG=$(mktemp "${TMPDIR:-/tmp}/cargo_build_XXXXXX")
trap 'rm -f "$BUILD_LOG"' EXIT

# ── Header ───────────────────────────────────────────────────────────────────
HR="━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

echo "$HR"
echo "RUST COMPILE TIME AUDIT"
echo "Date     : $(date -u '+%Y-%m-%d %H:%M:%S UTC')"
echo "Toolchain: $(rustc --version)"
echo "Cargo    : $(cargo --version)"
echo "Profile  : $PROFILE_DISPLAY"
echo "Target   : $TARGET_DISPLAY"
echo "$HR"

# ── 1. Workspace structure ───────────────────────────────────────────────────
echo ""
echo "▶ WORKSPACE MEMBERS"
if [[ -f Cargo.toml ]]; then
  cargo metadata --no-deps --format-version 1 2>/dev/null \
    | python3 "$SCRIPT_DIR/list_workspace_members.py" 2>/dev/null \
    || echo "  (failed to list workspace members)"
else
  echo "  (no Cargo.toml found in current directory)"
fi

# ── 2. Dependency count ──────────────────────────────────────────────────────
echo ""
echo "▶ DEPENDENCY SUMMARY"
DEP_COUNT=$(cargo tree --depth 1 ${WORKSPACE_ARGS[@]+"${WORKSPACE_ARGS[@]}"} ${PACKAGE_ARGS[@]+"${PACKAGE_ARGS[@]}"} 2>/dev/null | grep -c '── ' || true)
TOTAL_COUNT=$(cargo tree ${WORKSPACE_ARGS[@]+"${WORKSPACE_ARGS[@]}"} ${PACKAGE_ARGS[@]+"${PACKAGE_ARGS[@]}"} 2>/dev/null | grep -c '── ' || true)
echo "  Direct deps : $DEP_COUNT"
echo "  Total deps  : $TOTAL_COUNT"

DUP_COUNT=$(cargo tree -d ${WORKSPACE_ARGS[@]+"${WORKSPACE_ARGS[@]}"} ${PACKAGE_ARGS[@]+"${PACKAGE_ARGS[@]}"} 2>/dev/null | grep -c '^\[' || true)
if [[ $DUP_COUNT -gt 0 ]]; then
  echo "  ⚠ Duplicate crate versions: $DUP_COUNT"
  echo ""
  echo "  Duplicates:"
  cargo tree -d ${WORKSPACE_ARGS[@]+"${WORKSPACE_ARGS[@]}"} ${PACKAGE_ARGS[@]+"${PACKAGE_ARGS[@]}"} 2>/dev/null | head -30 | sed 's/^/    /'
else
  echo "  ✓ No duplicate crate versions detected"
fi

# ── 3. Timed build ──────────────────────────────────────────────────────────
echo ""
echo "▶ TIMED BUILD  (cargo build --timings)"
if [[ $SKIP_CLEAN -eq 0 ]]; then
  echo "  Cleaning first..."
  cargo clean 2>/dev/null
else
  echo "  Skipping clean (--no-clean)"
fi

cargo build ${WORKSPACE_ARGS[@]+"${WORKSPACE_ARGS[@]}"} ${PACKAGE_ARGS[@]+"${PACKAGE_ARGS[@]}"} ${RELEASE_ARGS[@]+"${RELEASE_ARGS[@]}"} \
  --timings --message-format=json 2>&1 \
  | tee "$BUILD_LOG" \
  | grep -E '(^error|Compiling|Finished|cargo-timing)' || true

TIMINGS_HTML="target/cargo-timings/cargo-timing.html"
[[ -f "$TIMINGS_HTML" ]] || TIMINGS_HTML=""

echo ""
echo "  Build output summary:"
grep -E '(Compiling|Finished|error)' "$BUILD_LOG" | tail -20 | sed 's/^/    /'

if [[ -n "$TIMINGS_HTML" ]]; then
  echo ""
  echo "  ✓ Timings report: $TIMINGS_HTML"

  # Use parse_timings.py to show top 15 slowest crates from the timings report.
  if command -v python3 &>/dev/null; then
    echo ""
    echo "  Top 15 slowest crates (from timings report):"
    python3 "$SCRIPT_DIR/parse_timings.py" --top 15 --compact 2>/dev/null \
      || echo "  (could not parse timings report)"
  fi
fi

# ── 4. Proc-macro crates ────────────────────────────────────────────────────
echo ""
echo "▶ PROC-MACRO DEPENDENCIES  (potential compile-time amplifiers)"
cargo tree ${WORKSPACE_ARGS[@]+"${WORKSPACE_ARGS[@]}"} ${PACKAGE_ARGS[@]+"${PACKAGE_ARGS[@]}"} 2>/dev/null \
  | grep -i 'proc.macro\|derive\|macro' \
  | grep -v '^#' \
  | sort -u \
  | head -20 \
  | sed 's/^/  /' \
  || echo "  (none detected or cargo tree failed)"

# ── 5. cargo-llvm-lines (if installed) ──────────────────────────────────────
echo ""
echo "▶ LLVM IR SIZE  (cargo-llvm-lines)"
if command -v cargo-llvm-lines &>/dev/null || cargo llvm-lines --version &>/dev/null 2>&1; then
  echo "  Top 20 generic functions by LLVM IR lines:"
  cargo llvm-lines ${PACKAGE_ARGS[@]+"${PACKAGE_ARGS[@]}"} ${RELEASE_ARGS[@]+"${RELEASE_ARGS[@]}"} 2>/dev/null \
    | head -22 | sed 's/^/  /' \
    || echo "  (cargo-llvm-lines failed — run manually)"
else
  echo "  ⚠ cargo-llvm-lines not installed."
  echo "    Install: cargo install cargo-llvm-lines"
  echo "    Then re-run this audit."
fi

# ── 6. Summary ──────────────────────────────────────────────────────────────
echo ""
echo "$HR"
echo "SUMMARY"
echo "$HR"
echo "  Timings HTML  : ${TIMINGS_HTML:-(not found)}"
echo "  Total deps    : $TOTAL_COUNT"
echo "  Duplicates    : $DUP_COUNT"
echo ""
echo "NEXT STEPS"
echo "  1. Open $TIMINGS_HTML in a browser to see the Gantt chart"
echo "  2. Look for the widest bar — that crate is your primary target"
echo "  3. If it's a dep: consider optional features or a lighter alternative"
echo "  4. If it's your code: run cargo-llvm-lines on it to find generic bloat"
echo "  5. If link time dominates: check linker setup (see SKILL.md Level 6)"
echo "$HR"
