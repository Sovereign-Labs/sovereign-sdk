#!/usr/bin/env bash
# Drift guard for switcheroo presets.
#
# Asserts that every workspace member (per `cargo metadata`) is referenced by
# at least one preset under scripts/switcheroo/presets/ (excluding `_backup`)
# OR by the explicit exclusion list at scripts/switcheroo/excluded.json.
#
# Also asserts that no preset/excluded entry points at a path that is not a
# workspace member (catches typos like the stale `sov-soak-lib`).
#
# Fails with a non-zero exit code (and an actionable message) if either
# invariant is violated. Intended to run as a pre-flight step in the cargo-hack
# CI jobs so a newly added crate cannot be silently omitted from feature-
# powerset coverage.

set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
PRESETS_DIR="$ROOT/scripts/switcheroo/presets"
EXCLUDED_FILE="$ROOT/scripts/switcheroo/excluded.json"

if ! command -v jq >/dev/null 2>&1; then
	echo "error: jq is required" >&2
	exit 2
fi

# Workspace members (relative to workspace root, no trailing /Cargo.toml).
mapfile -t workspace_members < <(
	cargo metadata --no-deps --format-version 1 --manifest-path "$ROOT/Cargo.toml" \
		| jq -r --arg root "$ROOT" '
			.packages[]
			| .manifest_path
			| sub("^" + $root + "/"; "")
			| sub("/Cargo.toml$"; "")
		' \
		| sort -u
)

# Union of all preset entries (skip _backup.json — it is not a real preset).
mapfile -t covered < <(
	for preset in "$PRESETS_DIR"/*.json; do
		case "$(basename "$preset")" in
			_*.json) continue ;;
		esac
		jq -r '.[]' "$preset"
	done | sort -u
)

# Excluded entries (always allowlisted).
if [[ -f "$EXCLUDED_FILE" ]]; then
	mapfile -t excluded < <(jq -r '.[]' "$EXCLUDED_FILE" | sort -u)
else
	excluded=()
fi

accounted_for=$(printf '%s\n' "${covered[@]}" "${excluded[@]}" | sort -u)

# Violation 1: a workspace member that is neither covered nor excluded.
missing=$(comm -23 <(printf '%s\n' "${workspace_members[@]}") <(printf '%s\n' "$accounted_for"))

# Violation 2: a preset/excluded entry that does not correspond to a workspace
# member (stale path or typo).
stale_sources=$(printf '%s\n' "${covered[@]}" "${excluded[@]}" | sort -u)
stale=$(comm -23 <(printf '%s\n' "$stale_sources") <(printf '%s\n' "${workspace_members[@]}"))

status=0

if [[ -n "$missing" ]]; then
	echo "error: workspace members not covered by any preset or excluded:" >&2
	while IFS= read -r path; do
		echo "  - $path" >&2
	done <<<"$missing"
	echo >&2
	echo "Add each to scripts/switcheroo/presets/<domain>.json, or to" >&2
	echo "scripts/switcheroo/excluded.json if it should not be checked by hack." >&2
	status=1
fi

if [[ -n "$stale" ]]; then
	echo "error: preset or excluded.json entries reference non-existent workspace members:" >&2
	while IFS= read -r path; do
		echo "  - $path" >&2
	done <<<"$stale"
	echo >&2
	echo "Fix the typo, or remove the entry if the crate has been deleted." >&2
	status=1
fi

if [[ $status -eq 0 ]]; then
	echo "preset coverage: OK (${#workspace_members[@]} workspace members accounted for)"
fi

exit $status
