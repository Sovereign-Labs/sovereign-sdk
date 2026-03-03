#!/usr/bin/env python3
"""Check JSON-RPC readiness for a backend endpoint."""

from __future__ import annotations

import json
import sys
from urllib import request


def main() -> int:
    if len(sys.argv) < 2 or len(sys.argv) > 3:
        print("Usage: wait_for_rpc.py <rpc_url> [timeout_seconds]", file=sys.stderr)
        return 2

    url = sys.argv[1]
    timeout = 0.5
    if len(sys.argv) == 3:
        try:
            timeout = float(sys.argv[2])
        except ValueError:
            return 1

    payload = json.dumps(
        {"jsonrpc": "2.0", "id": 1, "method": "eth_chainId", "params": []}
    ).encode("utf-8")
    req = request.Request(
        url,
        data=payload,
        headers={"Content-Type": "application/json"},
        method="POST",
    )

    try:
        with request.urlopen(req, timeout=timeout) as response:
            body = response.read()
        parsed = json.loads(body.decode("utf-8"))
    except Exception:
        return 1

    if not isinstance(parsed, dict):
        return 1
    if parsed.get("jsonrpc") != "2.0" or parsed.get("id") != 1:
        return 1
    if "error" in parsed:
        return 1
    if "result" not in parsed:
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
