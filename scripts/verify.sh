#!/usr/bin/env bash
# Copyright 2026 Petri Koistinen. Licensed under the Apache License, Version 2.0.
# Run every format/lint check and automated test suite, then print one
# status line per check. A check whose tool or prerequisite is missing is
# UNAVAILABLE and fails the run; --allow-unavailable reports it as SKIPPED
# and lets the run pass. Output of a failing check is printed after the
# summary; passing output is discarded.
set -uo pipefail

usage() {
  echo "usage: scripts/verify.sh [--allow-unavailable]" >&2
  exit 2
}

allow_unavailable=false
for arg in "$@"; do
  case "$arg" in
    --allow-unavailable) allow_unavailable=true ;;
    *) usage ;;
  esac
done

root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd -- "$root" || exit 1

log_dir="$(mktemp -d)"
trap 'rm -rf -- "$log_dir"' EXIT

rust_crates=(relay proxy-cli wasm-requester)
names=()
results=()
failed_logs=()

record() {
  names+=("$1")
  results+=("$2")
}

unavailable() {
  local name="$1" reason="$2"
  if "$allow_unavailable"; then
    record "$name" "SKIPPED ($reason)"
  else
    record "$name" "UNAVAILABLE ($reason)"
  fi
}

run() {
  local name="$1"
  shift
  local log="$log_dir/${#names[@]}.log"
  if "$@" >"$log" 2>&1; then
    record "$name" "PASS"
  else
    record "$name" "FAIL"
    failed_logs+=("$name|$log")
  fi
}

# Path dependencies of a crate that are missing on disk (e.g. a sibling
# refineid-core checkout).
missing_path_deps() {
  local crate="$1" dep
  sed -n 's/.*{.*path *= *"\([^"]*\)".*/\1/p' "$crate/Cargo.toml" | while read -r dep; do
    [[ -e "$crate/$dep/Cargo.toml" ]] || echo "$dep"
  done
}

# Dependencies of a crate that come from the sibling refineid-core checkout.
core_deps() {
  sed -n 's/^\([A-Za-z0-9_-]*\) *= *{.*path *= *"[^"]*refineid-core[^"]*".*/\1/p' "$1/Cargo.toml"
}

# Moves the refineid-core entries of a crate's Cargo.lock to the current core
# checkout; every other locked dependency stays as committed.
refresh_core_lock() {
  local manifest="$1/Cargo.toml" dep
  for dep in $(core_deps "$1"); do
    cargo update -q --manifest-path "$manifest" -p "$dep" || return 1
  done
}

# JavaScript and HTML formatting.
if [[ -x node_modules/.bin/prettier ]]; then
  run "prettier" node_modules/.bin/prettier --check .
else
  unavailable "prettier" "run npm ci"
fi

# Python lint and formatting.
ruff_bin=""
if [[ -x .venv/bin/ruff ]]; then
  ruff_bin=.venv/bin/ruff
elif command -v ruff >/dev/null 2>&1; then
  ruff_bin=ruff
fi
if [[ -n "$ruff_bin" ]]; then
  run "ruff check" "$ruff_bin" check .
  run "ruff format" "$ruff_bin" format --check .
else
  unavailable "ruff check" "ruff not installed"
  unavailable "ruff format" "ruff not installed"
fi

# Shell scripts.
if command -v shellcheck >/dev/null 2>&1; then
  shell_files=()
  while IFS= read -r f; do
    [[ -f "$f" ]] && shell_files+=("$f")
  done < <(git ls-files -co --exclude-standard '*.sh')
  run "shellcheck" shellcheck -S warning "${shell_files[@]}"
else
  unavailable "shellcheck" "shellcheck not installed"
fi

# Node suites.
if command -v node >/dev/null 2>&1; then
  run "protocol tests" node --test tests/*.test.mjs
  run "webusb harness" node tests/webusb-harness.js
else
  unavailable "protocol tests" "node not installed"
  unavailable "webusb harness" "node not installed"
fi

# mTLS self-test against the local throwaway test PKI.
if ! command -v python3 >/dev/null 2>&1; then
  unavailable "mtls selftest" "python3 not installed"
elif ! python3 -c 'import sys; sys.exit(sys.version_info < (3, 10))'; then
  unavailable "mtls selftest" "python3 older than 3.10"
elif [[ ! -f poc-mtls/pki/client.pem || ! -f poc-mtls/pki/server.pem ]]; then
  unavailable "mtls selftest" "no test PKI, run poc-mtls/mkcerts.sh"
else
  run "mtls selftest" python3 poc-mtls/selftest.py
fi

# Rust crates. RefineID tracks refineid-core main: the core entries of each
# Cargo.lock are refreshed first, then --locked holds everything else.
for crate in "${rust_crates[@]}"; do
  if ! command -v cargo >/dev/null 2>&1; then
    unavailable "$crate" "cargo not installed"
    continue
  fi
  missing="$(missing_path_deps "$crate")"
  if [[ -n "$missing" ]]; then
    unavailable "$crate" "missing path dependency $(echo "$missing" | head -n 1)"
    continue
  fi
  manifest="$crate/Cargo.toml"
  if [[ -n "$(core_deps "$crate")" ]]; then
    run "$crate core lock" refresh_core_lock "$crate"
  fi
  run "$crate fmt" cargo fmt --manifest-path "$manifest" --check
  run "$crate clippy" cargo clippy -q --locked --manifest-path "$manifest" --all-targets \
    -- -D warnings
  run "$crate test" cargo test -q --locked --manifest-path "$manifest"
done

pass=0
fail=0
missing_count=0
skipped=0
echo
echo "== verify summary =="
for i in "${!names[@]}"; do
  printf '%-26s %s\n' "${names[$i]}" "${results[$i]}"
  case "${results[$i]}" in
    PASS) pass=$((pass + 1)) ;;
    FAIL) fail=$((fail + 1)) ;;
    UNAVAILABLE*) missing_count=$((missing_count + 1)) ;;
    SKIPPED*) skipped=$((skipped + 1)) ;;
  esac
done
echo "$pass passed, $fail failed, $missing_count unavailable, $skipped skipped"

for entry in "${failed_logs[@]+"${failed_logs[@]}"}"; do
  echo
  echo "== ${entry%%|*} output =="
  cat -- "${entry#*|}"
done

if ((fail > 0 || missing_count > 0)); then
  exit 1
fi
