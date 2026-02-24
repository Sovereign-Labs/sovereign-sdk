#!/usr/bin/env python3
"""Minimal Engine API stub for Hive rpc-compat startup.

This intentionally supports only setup-critical methods for the first pass.
"""

from __future__ import annotations

import json
import os
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

ZERO_HASH = "0x" + ("0" * 64)
ZERO_PAYLOAD_ID = "0x" + ("0" * 16)


def rpc_error(req_id, code: int, message: str):
    return {"jsonrpc": "2.0", "id": req_id, "error": {"code": code, "message": message}}


def handle_single(request_obj):
    if not isinstance(request_obj, dict):
        return rpc_error(None, -32600, "Invalid Request")

    req_id = request_obj.get("id")
    method = request_obj.get("method")

    if method == "engine_forkchoiceUpdatedV3":
        return {
            "jsonrpc": "2.0",
            "id": req_id,
            "result": {
                "payloadStatus": {
                    "status": "VALID",
                    "latestValidHash": ZERO_HASH,
                    "validationError": None,
                },
                "payloadId": ZERO_PAYLOAD_ID,
            },
        }

    if method == "engine_exchangeCapabilities":
        return {"jsonrpc": "2.0", "id": req_id, "result": []}

    return rpc_error(req_id, -32601, f"Method {method} not supported by engine stub")


def handle_rpc(payload):
    if isinstance(payload, list):
        return [handle_single(item) for item in payload]
    return handle_single(payload)


class EngineHandler(BaseHTTPRequestHandler):
    server_version = "sov-hive-engine-stub/0.1"

    def log_message(self, fmt, *args):
        print(f"[engine-stub] {self.address_string()} - {fmt % args}")

    def do_POST(self):
        length = int(self.headers.get("Content-Length", "0"))
        raw = self.rfile.read(length) if length > 0 else b""

        try:
            payload = json.loads(raw.decode("utf-8")) if raw else {}
            response = handle_rpc(payload)
            body = json.dumps(response).encode("utf-8")
            self.send_response(200)
        except Exception as exc:
            body = json.dumps(rpc_error(None, -32700, f"Parse error: {exc}")).encode("utf-8")
            self.send_response(200)

        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)


def main() -> int:
    host = os.environ.get("ENGINE_STUB_HOST", "0.0.0.0")
    port = int(os.environ.get("ENGINE_STUB_PORT", "8551"))

    server = ThreadingHTTPServer((host, port), EngineHandler)
    print(f"[engine-stub] listening on {host}:{port}")
    server.serve_forever()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
