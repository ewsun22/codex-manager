#!/bin/bash
set -euo pipefail
repo_root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$repo_root"
out="artifacts/release/licenses"
mkdir -p "$out"
npm pkg get dependencies devDependencies > "$out/npm-dependencies.json"
if command -v cargo-license >/dev/null 2>&1; then cargo license --json > "$out/cargo-licenses.json"; else echo "cargo-license 未安装；请在发布环境生成 Rust license inventory。" > "$out/cargo-license-unavailable.txt"; fi
cp THIRD_PARTY_NOTICES.md "$out/THIRD_PARTY_NOTICES.md"
