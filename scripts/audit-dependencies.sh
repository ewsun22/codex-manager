#!/bin/bash
set -euo pipefail
repo_root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$repo_root"
if ! command -v node >/dev/null 2>&1; then
  echo "Node.js 未安装，依赖审计不能完整执行。" >&2
  exit 1
fi
if ! command -v npm >/dev/null 2>&1; then
  echo "npm 未安装，依赖审计不能完整执行。" >&2
  exit 1
fi
if ! command -v cargo-audit >/dev/null 2>&1; then
  echo "cargo-audit 未安装，依赖审计不能完整执行。" >&2
  exit 1
fi
npm_audit_json="$(mktemp "${RUNNER_TEMP:-/tmp}/codex-manager-npm-audit.XXXXXX")"
audit_json="$(mktemp "${RUNNER_TEMP:-/tmp}/codex-manager-cargo-audit.XXXXXX")"
target_tree="$(mktemp "${RUNNER_TEMP:-/tmp}/codex-manager-cargo-tree.XXXXXX")"
cleanup() { rm -f -- "$npm_audit_json" "$audit_json" "$target_tree"; }
trap cleanup EXIT
npm_status=0
npm audit --json > "$npm_audit_json" || npm_status=$?
node scripts/verify-npm-audit.mjs "$npm_audit_json"
test "$npm_status" -eq 0
cargo_status=0
cargo audit --json > "$audit_json" || cargo_status=$?
cargo tree --locked --target aarch64-apple-darwin --prefix none --format '{p}' > "$target_tree"
node scripts/verify-cargo-audit.mjs \
  --audit "$audit_json" \
  --allowlist config/cargo-audit-allowlist.json \
  --target-tree "$target_tree"
test "$cargo_status" -eq 0
