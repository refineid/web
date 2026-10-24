#!/usr/bin/env python3
# Copyright 2026 Petri Koistinen. Licensed under the Apache License, Version 2.0.
"""mTLS login proof-of-concept: loopback HTTPS server requiring a client cert.

Stdlib only. Binds 127.0.0.1, requests a client certificate during the TLS
handshake, verifies it against the test CA in poc-mtls/pki/, and renders the
server-verified identity. The page itself never touches any card or key: the
*server* asserts who logged in. Production would trust the DVV/VRK CA chain
and enforce FINEID certificate policy (see RESEARCH.md).

Usage:
    ./poc-mtls/mkcerts.sh            # once: create the throwaway test PKI
    python3 poc-mtls/server.py        # serve https://localhost:8443
"""

import argparse
import hashlib
import http.server
import ssl
from pathlib import Path

PKI_DIR = Path(__file__).resolve().parent / "pki"


def build_server_context(pki_dir: Path = PKI_DIR) -> ssl.SSLContext:
    """TLS server context that *requires* a client cert chained to our CA."""
    ctx = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    ctx.minimum_version = ssl.TLSVersion.TLSv1_2
    ctx.load_cert_chain(certfile=str(pki_dir / "server.pem"), keyfile=str(pki_dir / "server.key"))
    ctx.load_verify_locations(cafile=str(pki_dir / "ca.pem"))
    ctx.verify_mode = ssl.CERT_REQUIRED  # handshake fails with no/bad cert
    return ctx


def peer_summary(cert_dict: dict | None, der: bytes | None) -> dict:
    """Flatten getpeercert() output into display fields. No private data."""
    info = {"verified": cert_dict is not None}
    if cert_dict is None:
        return info

    def first(rdn_seq, key):
        for rdn in rdn_seq:
            for attr, value in rdn:
                if attr == key:
                    return value
        return None

    subject = cert_dict.get("subject", ())
    issuer = cert_dict.get("issuer", ())
    info.update(
        {
            "subject_cn": first(subject, "commonName"),
            "subject": ", ".join(f"{k}={v}" for rdn in subject for k, v in rdn),
            "issuer": ", ".join(f"{k}={v}" for rdn in issuer for k, v in rdn),
            "serial": cert_dict.get("serialNumber"),
            "not_before": cert_dict.get("notBefore"),
            "not_after": cert_dict.get("notAfter"),
        }
    )
    if der:
        info["sha256"] = hashlib.sha256(der).hexdigest()
    return info


def render_page(info: dict) -> bytes:
    status = "LOGGED IN" if info.get("verified") else "NOT VERIFIED"
    rows = "".join(
        f"<tr><th>{k}</th><td><code>{info.get(k, '')}</code></td></tr>"
        for k in ("subject_cn", "subject", "issuer", "serial", "not_before", "not_after", "sha256")
    )
    return f"""<!DOCTYPE html>
<html lang="en"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>mTLS login PoC</title></head>
<body style="font-family:system-ui;max-width:44rem;margin:2rem auto;padding:0 1rem">
<h1>mTLS login proof-of-concept: {status}</h1>
<p>This identity was asserted by the <em>server</em> from the TLS client
certificate your browser presented. The page itself never touched a card,
a key, or a PIN. Test PKI only &mdash; nothing here chains to DVV/VRK.</p>
<table border="1" cellpadding="6" cellspacing="0">{rows}</table>
</body></html>
""".encode()


class Handler(http.server.BaseHTTPRequestHandler):
    server_version = "RefineID-mTLS-PoC/1.0"

    def do_GET(self):  # noqa: N802 - stdlib handler naming
        der = self.connection.getpeercert(binary_form=True)
        info = peer_summary(self.connection.getpeercert(), der)
        body = render_page(info)
        self.send_response(200)
        self.send_header("Content-Type", "text/html; charset=utf-8")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, fmt, *args):  # keep PoC output readable
        print(f"{self.address_string()} {fmt % args}")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--port", type=int, default=8443)
    args = parser.parse_args()
    if not (PKI_DIR / "ca.pem").exists():
        raise SystemExit("No PKI yet. Run ./poc-mtls/mkcerts.sh first.")
    httpd = http.server.ThreadingHTTPServer(("127.0.0.1", args.port), Handler)
    httpd.socket = build_server_context().wrap_socket(httpd.socket, server_side=True)
    print(f"Serving https://localhost:{args.port} (loopback only, client cert REQUIRED)")
    print(
        "Import poc-mtls/pki/client.p12 (password: refineid-poc) so the "
        "browser can present it; accept the loopback cert warning."
    )
    httpd.serve_forever()


if __name__ == "__main__":
    main()
