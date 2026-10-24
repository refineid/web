#!/usr/bin/env python3
# Copyright 2026 Petri Koistinen. Licensed under the Apache License, Version 2.0.
"""Self-test for the mTLS PoC server. No socket bind required.

Runs the real Handler + TLS context over socketpair:
1. client presents the test cert -> 200 + verified identity page;
2. client presents nothing -> TLS handshake fails (CERT_REQUIRED).

Usage: python3 poc-mtls/selftest.py
"""

import http.server
import socket
import ssl
import sys
import threading
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from server import Handler, PKI_DIR, build_server_context  # noqa: E402


class UnboundHTTPServer(http.server.ThreadingHTTPServer):
    """Real server object for Handler; never binds (sandbox-safe)."""

    def server_bind(self):
        pass

    def server_close(self):
        pass


def client_context(with_cert: bool) -> ssl.SSLContext:
    ctx = ssl.SSLContext(ssl.PROTOCOL_TLS_CLIENT)
    ctx.check_hostname = False
    ctx.verify_mode = ssl.CERT_NONE  # PoC: skip server-cert check (loopback)
    if with_cert:
        ctx.load_cert_chain(
            certfile=str(PKI_DIR / "client.pem"), keyfile=str(PKI_DIR / "client.key")
        )
    return ctx


def run_case(with_cert: bool) -> bytes:
    """Full TLS handshake + HTTP through the real Handler over socketpair."""
    plain_server, plain_client = socket.socketpair()
    server_ctx = build_server_context()
    errors = []

    def serve_one():
        try:
            tls_server = server_ctx.wrap_socket(plain_server, server_side=True)
            try:
                tls_server.do_handshake()
                httpd = UnboundHTTPServer(("127.0.0.1", 8443), Handler, bind_and_activate=False)
                Handler(tls_server, ("127.0.0.1", 0), httpd)
            finally:
                try:
                    tls_server.close()
                except OSError:
                    pass
        except Exception as e:  # noqa: BLE001 - surfaced to the caller
            errors.append(e)

    thread = threading.Thread(target=serve_one, daemon=True)
    thread.start()
    try:
        tls = client_context(with_cert).wrap_socket(plain_client, server_hostname="localhost")
        try:
            tls.do_handshake()
            tls.sendall(b"GET / HTTP/1.0\r\nHost: localhost\r\n\r\n")
            raw = b""
            while b"\r\n\r\n" not in raw:
                chunk = tls.recv(65536)
                if not chunk:
                    break
                raw += chunk
            head, _, rest = raw.partition(b"\r\n\r\n")
            length = 0
            for line in head.decode("latin1").splitlines()[1:]:
                if line.lower().startswith("content-length:"):
                    length = int(line.split(":", 1)[1].strip())
            body = rest
            while len(body) < length:
                chunk = tls.recv(65536)
                if not chunk:
                    break
                body += chunk
            thread.join(timeout=10)
            return head + b"\r\n\r\n" + body
        finally:
            try:
                tls.close()
            except OSError:
                pass
    finally:
        thread.join(timeout=10)
        if errors and not with_cert:
            raise errors[0]


def main() -> None:
    failures = 0

    body = run_case(with_cert=True).decode()
    ok = "200" in body.splitlines()[0] and "LOGGED IN" in body and "POC TEST USER" in body
    print(("PASS" if ok else "FAIL") + " login with client cert -> 200 + verified identity")
    failures += not ok

    try:
        run_case(with_cert=False)
        print("FAIL no-cert client was NOT rejected")
        failures += 1
    except (ssl.SSLError, ConnectionResetError, BrokenPipeError) as e:
        print(f"PASS no-cert client rejected during handshake ({type(e).__name__})")

    print("ALL PASS" if not failures else f"{failures} FAILURES")
    sys.exit(1 if failures else 0)


if __name__ == "__main__":
    main()
