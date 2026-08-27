#!/bin/bash
set -euo pipefail
repo_root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$repo_root"
npm audit --omit=dev --audit-level=high
if command -v cargo-audit >/dev/null 2>&1; then cargo audit; else echo "cargo-audit 未安装，跳过 Rust advisory audit。"; fi
