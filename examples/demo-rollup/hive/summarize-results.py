#!/usr/bin/env python3
"""
Summarize Hive rpc-compat test results.

Usage:
    python3 summarize-results.py <run-dir> [--scope <regex-file>]

Examples:
    python3 summarize-results.py ~/workspace/hive/workspace/logs/full-20260402-161943-p0-nonhistorical
    python3 summarize-results.py ~/workspace/hive/workspace/logs/full-20260402-161943-p0-nonhistorical \
        --scope examples/demo-rollup/hive/p0-nonhistorical-tests.regex
"""

import argparse
import json
import os
import re
import sys
from pathlib import Path


def load_result_json(run_dir: Path) -> tuple[Path, dict]:
    """Find and load the result JSON (not hive.json) from the run directory."""
    candidates = [
        f for f in run_dir.iterdir()
        if f.suffix == ".json" and f.name != "hive.json"
    ]
    if not candidates:
        print(f"No result JSON found in {run_dir}", file=sys.stderr)
        sys.exit(1)
    path = candidates[0]
    with open(path) as f:
        return path, json.load(f)


def load_scope_regex(scope_file: Path) -> str | None:
    """Build a regex from a scope file (one pattern per line, # comments)."""
    lines = []
    with open(scope_file) as f:
        for line in f:
            line = line.strip()
            if line and not line.startswith("#"):
                lines.append(line)
    if not lines:
        return None
    return "^(" + "|".join(lines) + ")$"


def find_details_log(run_dir: Path) -> Path | None:
    """Find the simulator details log file."""
    details_dir = run_dir / "details"
    if not details_dir.is_dir():
        return None
    logs = list(details_dir.glob("*.log"))
    return logs[0] if logs else None


def extract_test_log(details_log: Path, begin: int, end: int) -> str:
    """Extract a test's log section by byte offsets."""
    with open(details_log, "rb") as f:
        f.seek(begin)
        return f.read(end - begin).decode("utf-8", errors="replace")


def strip_client_suffix(name: str) -> str:
    """Remove ' (client-name)' suffix from test names."""
    return re.sub(r"\s+\([^)]+\)$", "", name)


def main():
    parser = argparse.ArgumentParser(description="Summarize Hive rpc-compat results")
    parser.add_argument("run_dir", type=Path, help="Hive run directory")
    parser.add_argument("--scope", type=Path, help="Scope regex file to filter tests")
    args = parser.parse_args()

    run_dir = args.run_dir.expanduser().resolve()
    if not run_dir.is_dir():
        print(f"Run directory not found: {run_dir}", file=sys.stderr)
        sys.exit(1)

    result_path, data = load_result_json(run_dir)
    details_log = find_details_log(run_dir)

    scope_regex = None
    if args.scope:
        scope_regex = load_scope_regex(args.scope)

    cases = data.get("testCases", {})

    # Build sorted test list
    tests = []
    for tc in cases.values():
        name = strip_client_suffix(tc["name"])
        passed = tc.get("summaryResult", {}).get("pass", False)
        log_range = tc.get("summaryResult", {}).get("log")
        description = tc.get("description", "").strip()

        if scope_regex and not re.match(scope_regex, name):
            continue

        tests.append({
            "name": name,
            "passed": passed,
            "description": description,
            "log_begin": log_range["begin"] if log_range else None,
            "log_end": log_range["end"] if log_range else None,
        })

    tests.sort(key=lambda t: (t["passed"], t["name"]))

    total = len(tests)
    passed = sum(1 for t in tests if t["passed"])
    failed = total - passed

    # Print header
    scope_label = f" (scoped to {args.scope.name})" if args.scope else ""
    print(f"# Hive rpc-compat results{scope_label}")
    print(f"# Run: {run_dir}")
    print(f"# total={total}  pass={passed}  fail={failed}")
    print()

    # Group by method
    methods: dict[str, list] = {}
    for t in tests:
        method = t["name"].split("/")[0] if "/" in t["name"] else t["name"]
        methods.setdefault(method, []).append(t)

    # Print bullet summary
    print("## Summary")
    print()
    for method in sorted(methods):
        method_tests = methods[method]
        method_pass = sum(1 for t in method_tests if t["passed"])
        method_total = len(method_tests)
        status = "PASS" if method_pass == method_total else "FAIL"
        print(f"  [{status}] {method} ({method_pass}/{method_total})")
        for t in sorted(method_tests, key=lambda t: t["name"]):
            mark = "pass" if t["passed"] else "FAIL"
            print(f"         {mark}  {t['name']}")
    print()

    # Print failure details
    failures = [t for t in tests if not t["passed"]]
    if not failures:
        print("All tests passed.")
        return

    print(f"## Failure details ({len(failures)} tests)")
    print()

    for t in failures:
        print(f"### {t['name']}")
        if t["description"]:
            print(f"    {t['description']}")

        if details_log and t["log_begin"] is not None:
            log_text = extract_test_log(details_log, t["log_begin"], t["log_end"])
            # Show the request/response and diff, trimmed to reasonable length
            lines = log_text.strip().split("\n")
            for line in lines[:40]:
                print(f"    {line}")
            if len(lines) > 40:
                print(f"    ... ({len(lines) - 40} more lines)")
            print(f"    [log: {details_log.name} bytes {t['log_begin']}-{t['log_end']}]")
        else:
            print("    (no log details available)")

        print()

    print(f"# Result JSON: {result_path}")
    if details_log:
        print(f"# Details log: {details_log}")


if __name__ == "__main__":
    main()
