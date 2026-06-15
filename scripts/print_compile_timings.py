#!/usr/bin/env python3
"""Parse cargo --timings HTML report and print a summary table.

IMPORTANT - what this report does NOT measure
=============================================
cargo --timings only records units that were *actually recompiled* during
this invocation. Units that were served from the cache (fresh) do not
appear in the report at all.

That means:
  * The count below is "units recompiled in this run", NOT "total units
    in the build".
  * A small recompiled count with mostly-workspace crates near the top is
    a sign the cache is WARM for external dependencies. It is NOT a sign
    the cache is cold.
  * To get a real fresh-vs-dirty count, parse cargo's JSON message stream
    (`--message-format=json`) and look for `fresh: true/false` on each
    `compiler-artifact` event. This script intentionally does not do that
    to keep the CI step lightweight.

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
    # Every entry in UNIT_DATA was recompiled in this run. Fresh units are
    # not included by cargo, so we don't try to count them here; see module
    # docstring for why.
    recompiled_units = len(units)

    lines = []
    lines.append(f"## Compilation Timing: {job_name}")
    lines.append("")
    lines.append("| Metric | Value |")
    lines.append("|--------|-------|")
    lines.append(f"| Wall clock (bash SECONDS) | {wall_clock}s |")
    if cargo_duration is not None:
        lines.append(f"| Wall clock (cargo DURATION) | {cargo_duration}s |")
    lines.append(f"| Cumulative recompile time | {cumulative:.1f}s |")
    lines.append(f"| Units recompiled this run | {recompiled_units} |")
    lines.append("")
    lines.append(
        "> This report lists only the units that cargo recompiled. Units"
        " served from the cache are not shown. A short list of mostly-"
        "workspace crates here means the dependency cache is working;"
        " the workspace itself is always recompiled when its source"
        " changes."
    )
    lines.append("")
    lines.append("### Top 30 slowest recompiled crates")
    lines.append("")
    lines.append(f"| {'#':>3} | {'Crate':<40} | {'Version':<10} | {'Duration':>10} | {'Codegen':>10} | Target |")
    lines.append(f"|{'---':>5}|{'---':<42}|{'---':<12}|{'---':>12}|{'---':>12}|--------|")

    for i, u in enumerate(units[:30]):
        name = u.get("name", "?")
        version = u.get("version", "?")
        duration = u.get("duration", 0)
        rmeta = u.get("rmeta_time")
        codegen = duration - rmeta if rmeta is not None else 0
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
        f" cumulative_recompile={cumulative:.1f}s"
        f" recompiled_units={recompiled_units}"
    )

    output = "\n".join(lines)
    print(output)

    summary_file = os.environ.get("GITHUB_STEP_SUMMARY")
    if summary_file:
        with open(summary_file, "a") as f:
            f.write(output + "\n")


if __name__ == "__main__":
    main()
