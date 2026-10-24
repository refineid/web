#!/usr/bin/env bash
# Copyright 2026 Petri Koistinen. Licensed under the Apache License, Version 2.0.
# Test PKI for the mTLS login proof-of-concept. THROWAWAY identities only:
# a local test CA, a loopback server cert, and one test client cert.
# Nothing here chains to the Finnish DVV/VRK CA; production trust config
# is documented in RESEARCH.md. Never add this CA to a system trust store.
set -euo pipefail

pki_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/pki" >/dev/null 2>&1 && pwd || true)"
if [[ -z "$pki_dir" ]]; then
  mkdir -p "$(dirname -- "${BASH_SOURCE[0]}")/pki"
  pki_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/pki" && pwd)"
fi
cd -- "$pki_dir"

echo "== test CA =="
openssl req -x509 -newkey rsa:3072 -sha256 -days 825 -nodes \
  -keyout ca.key -out ca.pem -subj "/CN=RefineID mTLS PoC Test CA/O=RefineID PoC/C=FI"

echo "== loopback server cert =="
openssl req -newkey rsa:3072 -nodes -keyout server.key -out server.csr \
  -subj "/CN=localhost/O=RefineID PoC/C=FI"
printf 'subjectAltName=DNS:localhost,DNS:*.localhost,IP:127.0.0.1\nextendedKeyUsage=serverAuth\n' > server.ext
openssl x509 -req -in server.csr -CA ca.pem -CAkey ca.key -CAcreateserial \
  -days 825 -sha256 -extfile server.ext -out server.pem

echo "== test client cert (FINEID-shaped subject, synthetic identity) =="
openssl req -newkey rsa:3072 -nodes -keyout client.key -out client.csr \
  -subj "/C=FI/O=RefineID PoC/OU=Test users/CN=POC TEST USER 010190-123N"
printf 'extendedKeyUsage=clientAuth\n' > client.ext
openssl x509 -req -in client.csr -CA ca.pem -CAkey ca.key -CAcreateserial \
  -days 825 -sha256 -extfile client.ext -out client.pem

echo "== PKCS#12 bundle for browser/OS import (password: refineid-poc) =="
openssl pkcs12 -export -out client.p12 -inkey client.key -in client.pem \
  -certfile ca.pem -passout pass:refineid-poc -name "RefineID PoC Test User"

rm -f server.csr client.csr server.ext client.ext ca.srl
chmod 600 ca.key server.key client.key client.p12

echo "== verify chains =="
openssl verify -CAfile ca.pem server.pem client.pem
echo
echo "PKI ready in $pki_dir"
openssl x509 -in client.pem -noout -subject -issuer -serial
