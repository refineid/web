#!/usr/bin/env bash
# Copyright 2026 Petri Koistinen. Licensed under the Apache License, Version 2.0.
set -euo pipefail

repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
muse_binary="$(type -P muse || true)"
model_id="muse-spark-1.3-contributor"

if [[ -z "$muse_binary" ]]; then
  echo "Muse CLI is not available on PATH." >&2
  exit 1
fi
if [[ ! -s "$repo_root/PROMPT.md" ]]; then
  echo "PROMPT.md is missing or empty." >&2
  exit 1
fi
if [[ "${1:-}" == "--check" && $# -eq 1 ]]; then
  printf 'Workspace: %s\nMuse: %s\nModel: %s\nPrompt: ready\n' \
    "$repo_root" "$muse_binary" "$model_id"
  exit 0
fi
if [[ $# -ne 0 ]]; then
  echo "Usage: scripts/start-muse.sh [--check]" >&2
  exit 2
fi

cd -- "$repo_root"
exec "$muse_binary" --provider meta --model "$model_id" \
  --reasoning-effort high --workspace "$repo_root" --worktree off \
  "$(cat -- "$repo_root/PROMPT.md")"
