#!/bin/bash
set -euo pipefail
repo_root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$repo_root"
out="artifacts/release/sbom"
mkdir -p "$out"
npm sbom --sbom-format cyclonedx --sbom-type application > "$out/npm.cdx.json"
if command -v cargo-cyclonedx >/dev/null 2>&1; then
  generated_files=()
  cleanup_generated() {
    for generated in "${generated_files[@]}"; do
      if [[ -f "$generated" ]]; then
        rm -f -- "$generated"
      fi
    done
  }
  trap cleanup_generated EXIT
  rm -f -- "$out/cargo-sbom-unavailable.txt"
  for entry in \
    "core:crates/codex-core/Cargo.toml" \
    "storage:crates/codex-storage/Cargo.toml" \
    "desktop:src-tauri/Cargo.toml"
  do
    name="${entry%%:*}"
    manifest="${entry#*:}"
    override=".codex-manager-sbom-${name}-$$"
    generated="$(dirname "$manifest")/${override}.json"
    generated_files+=("$generated")
    cargo cyclonedx \
      --manifest-path "$manifest" \
      --format json \
      --override-filename "$override"
    mv -- "$generated" "$out/cargo-${name}.cdx.json"
  done
  trap - EXIT
else
  echo "cargo-cyclonedx 未安装；跳过 Rust SBOM。" > "$out/cargo-sbom-unavailable.txt"
fi
manifest_tmp="$(mktemp "$out/.SHA256SUMS.XXXXXX")"
find "$out" -maxdepth 1 -type f ! -name 'SHA256SUMS' ! -name '.SHA256SUMS.*' -print0 | sort -z | xargs -0 shasum -a 256 > "$manifest_tmp"
mv "$manifest_tmp" "$out/SHA256SUMS"
