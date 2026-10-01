#!/usr/bin/env python3
"""List workspace members from `cargo metadata` JSON.

Usage:
    cargo metadata --no-deps --format-version 1 | python3 list_workspace_members.py
"""

import json
import os
import sys


def count_rs_files(manifest_path):
    """Count .rs files under the src/ directory next to a Cargo.toml."""
    src_dir = os.path.join(os.path.dirname(manifest_path), "src")
    if not os.path.isdir(src_dir):
        return 0
    count = 0
    for root, _dirs, files in os.walk(src_dir):
        count += sum(1 for f in files if f.endswith(".rs"))
    return count


def main():
    data = json.load(sys.stdin)
    packages_by_id = {p["id"]: p for p in data.get("packages", [])}

    for member_id in sorted(data.get("workspace_members", [])):
        pkg = packages_by_id.get(member_id)
        if not pkg:
            continue
        rs_count = count_rs_files(pkg.get("manifest_path", ""))
        print(f"  {pkg['name']:40s}  v{pkg['version']}  ({rs_count} .rs files)")


if __name__ == "__main__":
    main()
