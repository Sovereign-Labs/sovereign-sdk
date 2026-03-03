#!/usr/bin/env python3
"""Hive helper services: Engine API stub and root RPC proxy."""
from __future__ import annotations

import http.client
import json
import os
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import urlsplit

ZERO_HASH = "0x" + ("0" * 64)
ZERO_PAYLOAD_ID = "0x" + ("0" * 16)


def rpc_error(req_id, code: int, message: str):
    return {"jsonrpc": "2.0", "id": req_id, "error": {"code": code, "message": message}}


def parse_backend():
    backend = os.environ.get("SOV_HIVE_RPC_BACKEND_URL", "http://127.0.0.1:8546/rpc")
    parsed = urlsplit(backend)
    if parsed.scheme != "http" or not parsed.hostname:
        raise ValueError(f"Unsupported backend URL: {backend}")
    return parsed.hostname, parsed.port or 80, parsed.path or "/rpc"


def read_body(handler: BaseHTTPRequestHandler) -> bytes:
    length = int(handler.headers.get("Content-Length", "0"))
    return handler.rfile.read(length) if length > 0 else b""


def send_json(handler: BaseHTTPRequestHandler, status: int, payload) -> None:
    body = json.dumps(payload).encode("utf-8")
    handler.send_response(status)
    handler.send_header("Content-Type", "application/json")
    handler.send_header("Content-Length", str(len(body)))
    handler.end_headers()
    handler.wfile.write(body)


def handle_engine_single(request_obj):
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


def handle_engine_rpc(payload):
    if isinstance(payload, list):
        return [handle_engine_single(item) for item in payload]
    return handle_engine_single(payload)


class EngineHandler(BaseHTTPRequestHandler):
    server_version = "sov-hive-engine-stub/0.2"

    def log_message(self, fmt, *args):
        print(f"[engine-stub] {self.address_string()} - {fmt % args}")

    def do_POST(self):
        raw = read_body(self)
        try:
            payload = json.loads(raw.decode("utf-8")) if raw else {}
            send_json(self, 200, handle_engine_rpc(payload))
        except Exception as exc:  # noqa: BLE001
            send_json(self, 200, rpc_error(None, -32700, f"Parse error: {exc}"))


class ProxyHandler(BaseHTTPRequestHandler):
    server_version = "sov-hive-rpc-proxy/0.2"
    backend_host = "127.0.0.1"
    backend_port = 8546
    backend_path = "/rpc"

    def log_message(self, fmt, *args):
        print(f"[rpc-proxy] {self.address_string()} - {fmt % args}")

    def do_GET(self):
        self.send_response(200)
        self.send_header("Content-Type", "text/plain")
        self.end_headers()
        self.wfile.write(b"ok\n")

    def do_POST(self):
        path = self.backend_path if self.path in ("/", "") else self.path
        body = read_body(self)
        conn = None
        try:
            conn = http.client.HTTPConnection(self.backend_host, self.backend_port, timeout=30)
            conn.request(
                "POST",
                path,
                body=body,
                headers={"Content-Type": self.headers.get("Content-Type", "application/json")},
            )
            upstream = conn.getresponse()
            response_body = upstream.read()
            self.send_response(upstream.status)
            self.send_header("Content-Type", upstream.getheader("Content-Type", "application/json"))
            self.send_header("Content-Length", str(len(response_body)))
            self.end_headers()
            self.wfile.write(response_body)
        except Exception as exc:  # noqa: BLE001
            send_json(self, 502, rpc_error(None, -32000, f"rpc proxy backend error: {exc}"))
        finally:
            if conn is not None:
                conn.close()


def main() -> int:
    engine_host = os.environ.get("ENGINE_STUB_HOST", "0.0.0.0")
    engine_port = int(os.environ.get("ENGINE_STUB_PORT", "8551"))
    proxy_host = os.environ.get("SOV_HIVE_RPC_PROXY_HOST", "0.0.0.0")
    proxy_port = int(os.environ.get("SOV_HIVE_RPC_PROXY_PORT", "8545"))
    backend_host, backend_port, backend_path = parse_backend()
    ProxyHandler.backend_host = backend_host
    ProxyHandler.backend_port = backend_port
    ProxyHandler.backend_path = backend_path
    try:
        engine_server = ThreadingHTTPServer((engine_host, engine_port), EngineHandler)
        proxy_server = ThreadingHTTPServer((proxy_host, proxy_port), ProxyHandler)
    except OSError as exc:
        print(f"[hive-services] failed to bind service port: {exc}")
        return 1
    print(f"[engine-stub] listening on {engine_host}:{engine_port}")
    print(f"[rpc-proxy] listening on {proxy_host}:{proxy_port} -> {backend_host}:{backend_port}{backend_path}")
    threading.Thread(target=engine_server.serve_forever, name="engine-stub-server", daemon=True).start()
    try:
        proxy_server.serve_forever()
    except KeyboardInterrupt:
        pass
    finally:
        engine_server.shutdown()
        proxy_server.shutdown()
        engine_server.server_close()
        proxy_server.server_close()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
