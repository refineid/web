#!/usr/bin/env bash
# Copyright 2026 Petri Koistinen. Licensed under the Apache License, Version 2.0.
set -euo pipefail

repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
if [[ ! -f "$repo_root/public/index.html" ]]; then
  echo "No public/index.html yet. Run the prototype agent first." >&2
  exit 1
fi
exec python3 -m http.server "${1:-8080}" --bind 127.0.0.1 \
  --directory "$repo_root/public"
