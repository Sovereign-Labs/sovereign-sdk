#!/usr/bin/env python3
"""Tiny HTTP proxy that maps Hive root RPC requests to the rollup /rpc endpoint."""

from __future__ import annotations

import http.client
import os
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import urlsplit


def parse_backend():
    backend = os.environ.get("SOV_HIVE_RPC_BACKEND_URL", "http://127.0.0.1:8546/rpc")
    parsed = urlsplit(backend)
    if parsed.scheme != "http" or not parsed.hostname:
        raise ValueError(f"Unsupported backend URL: {backend}")
    return parsed.hostname, parsed.port or 80, parsed.path or "/rpc"


class ProxyHandler(BaseHTTPRequestHandler):
    server_version = "sov-hive-rpc-proxy/0.1"

    def log_message(self, fmt, *args):
        print(f"[rpc-proxy] {self.address_string()} - {fmt % args}")

    def do_GET(self):
        self.send_response(200)
        self.send_header("Content-Type", "text/plain")
        self.end_headers()
        self.wfile.write(b"ok\n")

    def do_POST(self):
        backend_host, backend_port, backend_path = parse_backend()
        path = backend_path if self.path == "/" else self.path
        if path == "":
            path = backend_path

        length = int(self.headers.get("Content-Length", "0"))
        body = self.rfile.read(length) if length > 0 else b""

        headers = {"Content-Type": self.headers.get("Content-Type", "application/json")}

        try:
            conn = http.client.HTTPConnection(backend_host, backend_port, timeout=30)
            conn.request("POST", path, body=body, headers=headers)
            upstream = conn.getresponse()
            response_body = upstream.read()
            self.send_response(upstream.status)
            self.send_header(
                "Content-Type", upstream.getheader("Content-Type", "application/json")
            )
            self.send_header("Content-Length", str(len(response_body)))
            self.end_headers()
            self.wfile.write(response_body)
        except Exception as exc:
            response = (
                '{'
                '"jsonrpc":"2.0",'
                '"id":null,'
                '"error":{"code":-32000,"message":"rpc proxy backend error: '
                + str(exc).replace('"', '\\"')
                + '"}'
                '}'
            ).encode("utf-8")
            self.send_response(502)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(response)))
            self.end_headers()
            self.wfile.write(response)


def main() -> int:
    host = os.environ.get("SOV_HIVE_RPC_PROXY_HOST", "0.0.0.0")
    port = int(os.environ.get("SOV_HIVE_RPC_PROXY_PORT", "8545"))
    server = ThreadingHTTPServer((host, port), ProxyHandler)
    print(f"[rpc-proxy] listening on {host}:{port}")
    server.serve_forever()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
