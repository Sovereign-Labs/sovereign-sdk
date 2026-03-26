#!/usr/bin/env python3
"""Parse cargo build timing data and print a sorted table of per-crate durations.

Supports both JSON (cargo-timing-*.json) and HTML (cargo-timing.html) reports.

Usage:
    python3 parse_timings.py                                        # auto-find latest timing report
    python3 parse_timings.py target/cargo-timings/cargo-timing-*.json
    python3 parse_timings.py target/cargo-timings/cargo-timing.html # HTML report
    python3 parse_timings.py --top 15 --compact                     # short table for audit.sh
    cargo build --timings --message-format=json 2>&1 | python3 parse_timings.py
"""

import argparse
import json
import os
import re
import sys


def parse_html_report(path):
    """Parse cargo --timings HTML report by extracting the UNIT_DATA JS variable."""
    with open(path) as f:
        html = f.read()

    duration_match = re.search(r"DURATION = (\d+)", html)
    wall_clock = int(duration_match.group(1)) if duration_match else 0

    match = re.search(r"const UNIT_DATA = (\[.*?\]);", html, re.DOTALL)
    if not match:
        print("Could not find UNIT_DATA in HTML report", file=sys.stderr)
        sys.exit(1)

    raw_units = json.loads(match.group(1))
    units = []
    for u in raw_units:
        units.append({
            "name": u.get("name", "?"),
            "kind": "lib",
            "fresh": False,
            "duration": u.get("duration"),
            "mode": "",
            "rmeta_time": u.get("rmeta_time"),
        })
    return units, wall_clock


def parse_stdin():
    """Parse compiler-artifact messages from stdin (no duration info available)."""
    units = []
    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue
        try:
            msg = json.loads(line)
        except json.JSONDecodeError:
            continue
        if msg.get("reason") == "compiler-artifact":
            units.append({
                "name": msg.get("target", {}).get("name", "?"),
                "kind": msg.get("target", {}).get("kind", ["?"])[0],
                "fresh": msg.get("fresh", False),
                "duration": None,
            })
    return units


def parse_timing_json(path):
    """Parse the NDJSON timing file from target/cargo-timings/."""
    units = []
    with open(path) as f:
        for line in f:
            line = line.strip()
            if not line:
                continue
            try:
                obj = json.loads(line)
            except json.JSONDecodeError:
                continue
            if obj.get("type") == "unit_result":
                target = obj.get("target", {})
                units.append({
                    "name": target.get("name", "?"),
                    "kind": target.get("kind", ["?"])[0] if target.get("kind") else "?",
                    "fresh": obj.get("fresh", False),
                    "duration": obj.get("duration"),
                    "mode": obj.get("mode", ""),
                })
    return units


def find_latest_timing_json():
    """Return the path to the most recent cargo-timing-*.json, or None."""
    timing_dir = "target/cargo-timings"
    if not os.path.isdir(timing_dir):
        return None
    json_files = sorted(
        (os.path.join(timing_dir, f) for f in os.listdir(timing_dir)
         if f.startswith("cargo-timing-") and f.endswith(".json")),
        key=os.path.getmtime,
        reverse=True,
    )
    return json_files[0] if json_files else None


def find_html_report():
    """Return the default HTML timing report path if it exists."""
    html_path = "target/cargo-timings/cargo-timing.html"
    return html_path if os.path.exists(html_path) else None


def print_compact(units, top):
    """Print a short crate/duration/mode table (used by audit.sh)."""
    if not units:
        print("  (no timing data found)")
        return
    units.sort(key=lambda u: -(u.get("duration") or 0))
    display = units[:top] if top else units
    print(f"  {'Crate':<40} {'Duration':>10}  Mode")
    print(f"  {'-'*40} {'-'*10}  ----")
    for u in display:
        name = u["name"][:40]
        dur = u.get("duration")
        mode = u.get("mode", "")
        if dur is not None:
            print(f"  {name:<40} {dur:>9.2f}s  {mode}")
        else:
            print(f"  {name:<40} {'?':>10}  {mode}")


def print_table(units, title="Per-Crate Build Times", top=None):
    """Print the full table with kind, percentage, status, and bottleneck bars."""
    if not units:
        print("No timing data found.")
        return

    # Sort by duration descending, put None last
    units.sort(key=lambda u: (u.get("duration") is None, -(u.get("duration") or 0)))

    total = sum(u["duration"] for u in units if u.get("duration") is not None)
    display = units[:top] if top else units

    print(f"\n{'='*70}")
    print(f"  {title}")
    print(f"  Total accounted: {total:.2f}s across {len(units)} units")
    print(f"{'='*70}")
    print(f"  {'Crate':<38} {'Kind':<12} {'Duration':>9}  {'%Total':>6}  Status")
    print(f"  {'-'*38} {'-'*12} {'-'*9}  {'-'*6}  ------")

    for u in display:
        name = u["name"][:38]
        kind = (u.get("kind") or "?")[:12]
        dur = u.get("duration")
        status = "fresh" if u.get("fresh", False) else "compiled"

        if dur is not None:
            pct = (dur / total * 100) if total > 0 else 0
            print(f"  {name:<38} {kind:<12} {dur:>8.2f}s  {pct:>5.1f}%  {status}")
        else:
            print(f"  {name:<38} {kind:<12} {'?':>9}  {'?':>6}  {status}")

    print(f"{'='*70}")
    print()

    # Top bottlenecks bar chart
    top5 = [u for u in units if u.get("duration") is not None][:5]
    if top5:
        print("  ⚡ Top bottlenecks:")
        for u in top5:
            dur = u["duration"]
            pct = dur / total * 100 if total > 0 else 0
            bar = "█" * min(int(pct / 2), 40)
            print(f"     {u['name']:<35} {dur:6.2f}s  {bar}")
        print()


def main():
    parser = argparse.ArgumentParser(description="Parse cargo build timing data.")
    parser.add_argument("files", nargs="*", help="Timing report file(s) to parse")
    parser.add_argument("--top", type=int, default=0, help="Show only the top N slowest crates")
    parser.add_argument("--compact", action="store_true", help="Short table (crate/duration/mode only)")
    args = parser.parse_args()

    top = args.top or None

    if args.files:
        for path in args.files:
            if not os.path.exists(path):
                print(f"File not found: {path}", file=sys.stderr)
                continue
            if path.endswith(".html"):
                units, wall_clock = parse_html_report(path)
                if args.compact:
                    print_compact(units, top)
                else:
                    print(f"Wall clock: {wall_clock}s")
                    print_table(units, title=f"Build Times (HTML): {os.path.basename(path)}", top=top)
            else:
                units = parse_timing_json(path)
                if args.compact:
                    print_compact(units, top)
                else:
                    print_table(units, title=f"Build Times: {os.path.basename(path)}", top=top)
    else:
        latest = find_latest_timing_json()
        if latest:
            if not args.compact:
                print(f"Using most recent timing file: {latest}")
            units = parse_timing_json(latest)
            if args.compact:
                print_compact(units, top)
            else:
                print_table(units, top=top)
        elif not sys.stdin.isatty():
            units = parse_stdin()
            if units:
                if args.compact:
                    print_compact(units, top)
                else:
                    print_table(units, title="Crates compiled (no timing data in message format)", top=top)
            else:
                html_path = find_html_report()
                if html_path:
                    if not args.compact:
                        print(f"Using HTML report: {html_path}")
                    units, wall_clock = parse_html_report(html_path)
                    if args.compact:
                        print_compact(units, top)
                    else:
                        print(f"Wall clock: {wall_clock}s")
                        print_table(units, top=top)
                else:
                    if args.compact:
                        print_compact(units, top)
                    else:
                        print_table(units, title="Crates compiled (no timing data in message format)", top=top)
        else:
            html_path = find_html_report()
            if html_path:
                print(f"Using HTML report: {html_path}")
                units, wall_clock = parse_html_report(html_path)
                print(f"Wall clock: {wall_clock}s")
                print_table(units, top=top)
            else:
                parser.print_help()
                sys.exit(1)


if __name__ == "__main__":
    main()
