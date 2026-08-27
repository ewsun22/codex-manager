#!/bin/bash
set -euo pipefail
repo_root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$repo_root"
./scripts/check-version-sync.sh
./scripts/preflight-macos.sh
release_dir="$repo_root/artifacts/release"
if [[ "$release_dir" != "$repo_root/artifacts/release" ]]; then
  echo "拒绝清理意外的发布目录。" >&2
  exit 1
fi
rm -rf -- "$release_dir"

# Remove old bundle directories before the build so a failed current build
# cannot accidentally package a stale .app or .dmg.
for generated_bundle in \
  "$repo_root/target/release/bundle" \
  "$repo_root/src-tauri/target/release/bundle"; do
  case "$generated_bundle" in
    "$repo_root/target/release/bundle"|"$repo_root/src-tauri/target/release/bundle")
      rm -rf -- "$generated_bundle"
      ;;
    *)
      echo "拒绝清理意外的 bundle 目录。" >&2
      exit 1
      ;;
  esac
done

npm ci
CI=true npm run tauri:build -- --bundles app,dmg
mkdir -p "$release_dir"
bundle_root=""
for candidate in target/release/bundle src-tauri/target/release/bundle; do
  if [[ -d "$candidate" ]]; then
    bundle_root="$candidate"
    break
  fi
done
if [[ -z "$bundle_root" ]]; then
  echo "未找到 Tauri bundle 目录。" >&2
  exit 1
fi
artifact_count=0
while IFS= read -r -d '' artifact; do
  ditto "$artifact" "$release_dir/$(basename "$artifact")"
  artifact_count=$((artifact_count + 1))
done < <(find "$bundle_root" -maxdepth 4 -type d -name '*.app' -print0)
while IFS= read -r -d '' artifact; do
  cp "$artifact" "$release_dir/"
  artifact_count=$((artifact_count + 1))
done < <(find "$bundle_root" -maxdepth 4 -type f -name '*.dmg' -print0)
if [[ "$artifact_count" -lt 2 ]] || \
  [[ "$(find "$release_dir" -maxdepth 1 -type d -name '*.app' | wc -l | tr -d ' ')" -lt 1 ]] || \
  [[ "$(find "$release_dir" -maxdepth 1 -type f -name '*.dmg' | wc -l | tr -d ' ')" -lt 1 ]]; then
  echo "Tauri bundle 必须同时产生 .app 和 .dmg。" >&2
  exit 1
fi
