#!/usr/bin/env python3
"""Parse cargo --timings HTML report and print a summary table.

Used by CI compile-monitoring jobs to produce machine-readable output.

Usage:
    WALL_CLOCK_SECONDS=342 python3 scripts/print_compile_timings.py timing_check
"""

import json
import os
import re
import sys


def main():
    if len(sys.argv) < 2:
        print("Usage: print_compile_timings.py <job_name>", file=sys.stderr)
        sys.exit(1)

    job_name = sys.argv[1]
    html_path = "target/cargo-timings/cargo-timing.html"

    with open(html_path) as f:
        html = f.read()

    # Extract wall-clock duration recorded by cargo itself
    duration_match = re.search(r"DURATION = (\d+)", html)
    cargo_duration = int(duration_match.group(1)) if duration_match else None

    # Extract per-unit timing data embedded in the HTML
    match = re.search(r"const UNIT_DATA = (\[.*?\]);", html, re.DOTALL)
    if not match:
        print("ERROR: Could not find UNIT_DATA in timing HTML", file=sys.stderr)
        sys.exit(1)

    units = json.loads(match.group(1))
    units.sort(key=lambda u: u.get("duration", 0), reverse=True)

    wall_clock = int(os.environ.get("WALL_CLOCK_SECONDS", 0))
    cumulative = sum(u.get("duration", 0) for u in units)
    total_units = len(units)
    fresh_units = sum(1 for u in units if u.get("duration", 0) == 0.0)
    dirty_units = total_units - fresh_units

    lines = []
    lines.append(f"## Compilation Timing: {job_name}")
    lines.append("")
    lines.append("| Metric | Value |")
    lines.append("|--------|-------|")
    lines.append(f"| Wall clock (bash SECONDS) | {wall_clock}s |")
    if cargo_duration is not None:
        lines.append(f"| Wall clock (cargo DURATION) | {cargo_duration}s |")
    lines.append(f"| Cumulative unit time | {cumulative:.1f}s |")
    lines.append(f"| Total units | {total_units} |")
    lines.append(f"| Dirty (recompiled) units | {dirty_units} |")
    lines.append(f"| Fresh (cached) units | {fresh_units} |")
    lines.append("")
    lines.append("### Top 30 slowest crates")
    lines.append("")
    lines.append(f"| {'#':>3} | {'Crate':<40} | {'Version':<10} | {'Duration':>10} | {'Codegen':>10} | Target |")
    lines.append(f"|{'---':>5}|{'---':<42}|{'---':<12}|{'---':>12}|{'---':>12}|--------|")

    for i, u in enumerate(units[:30]):
        name = u.get("name", "?")
        version = u.get("version", "?")
        duration = u.get("duration", 0)
        rmeta = u.get("rmeta_time", 0)
        codegen = duration - rmeta if rmeta else 0
        target = u.get("target", "")
        lines.append(
            f"| {i + 1:>3} | {name:<40} | {version:<10} | {duration:>8.1f}s | {codegen:>8.1f}s | {target} |"
        )

    lines.append("")
    lines.append("---")
    lines.append("")
    lines.append(
        f"COMPILATION_TIMING_SUMMARY: job={job_name}"
        f" wall_clock={wall_clock}s"
        f" cumulative={cumulative:.1f}s"
        f" units={total_units}"
        f" dirty={dirty_units}"
        f" fresh={fresh_units}"
    )

    output = "\n".join(lines)
    print(output)

    summary_file = os.environ.get("GITHUB_STEP_SUMMARY")
    if summary_file:
        with open(summary_file, "a") as f:
            f.write(output + "\n")


if __name__ == "__main__":
    main()
