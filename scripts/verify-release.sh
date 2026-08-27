#!/bin/bash
set -euo pipefail
repo_root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$repo_root"
label="${1:-unsigned}"
if [[ "$label" != "unsigned" && ! "$label" =~ ^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(-[0-9A-Za-z-]+(\.[0-9A-Za-z-]+)*)?$ ]]; then
  echo "release label 必须是 unsigned 或 SemVer 兼容版本标签。" >&2
  exit 1
fi
if [[ "$label" == "unsigned" ]]; then
  ./scripts/check-version-sync.sh
else
  ./scripts/check-version-sync.sh "$label"
fi
release_dir="artifacts/release"
test -f LICENSE; test -f NOTICE; test -f THIRD_PARTY_NOTICES.md; test -d "$release_dir"
app_count="$(find "$release_dir" -maxdepth 1 -type d -name '*.app' | wc -l | tr -d ' ')"
dmg_count="$(find "$release_dir" -maxdepth 1 -type f -name '*.dmg' | wc -l | tr -d ' ')"
if [[ "$app_count" -lt 1 || "$dmg_count" -lt 1 ]]; then
  echo "发布目录必须同时包含 .app 和 .dmg。" >&2
  exit 1
fi

required_release_files=(
  "$release_dir/sbom/npm.cdx.json"
  "$release_dir/sbom/cargo-core.cdx.json"
  "$release_dir/sbom/cargo-storage.cdx.json"
  "$release_dir/sbom/cargo-desktop.cdx.json"
  "$release_dir/sbom/SHA256SUMS"
  "$release_dir/licenses/cargo-licenses.json"
  "$release_dir/licenses/npm-dependencies.json"
  "$release_dir/licenses/THIRD_PARTY_NOTICES.md"
)
for required_file in "${required_release_files[@]}"; do
  if [[ ! -f "$required_file" ]]; then
    echo "发布目录缺少必需的 SBOM/许可证文件：$required_file" >&2
    exit 1
  fi
done
if find "$release_dir/sbom" "$release_dir/licenses" -type f -name '*-unavailable.txt' -print -quit | grep -q .; then
  echo "SBOM 或许可证清单不完整，拒绝验证发布目录。" >&2
  exit 1
fi
for inventory in \
  "$release_dir"/sbom/*.json \
  "$release_dir"/licenses/*.json; do
  node --input-type=module -e \
    'import { readFileSync } from "node:fs"; JSON.parse(readFileSync(process.argv[1], "utf8"));' \
    "$inventory"
done
shasum -a 256 -c "$release_dir/sbom/SHA256SUMS"

source_sha="$(git rev-parse --verify HEAD 2>/dev/null || true)"
if [[ -z "$source_sha" ]]; then
  source_sha="unversioned-working-tree"
fi
printf 'source=%s\nlabel=%s\nsigning=unsigned\n' \
  "$source_sha" "$label" > "$release_dir/BUILD-SOURCE-${label}.txt"

manifest_tmp="$(mktemp "$release_dir/.SHA256SUMS-${label}.XXXXXX")"
find "$release_dir" -type f ! -name 'SHA256SUMS-*' ! -name '.SHA256SUMS-*' -print0 | sort -z | xargs -0 shasum -a 256 > "$manifest_tmp"
mv "$manifest_tmp" "$release_dir/SHA256SUMS-${label}"
echo "Release files (${label}):"; find "$release_dir" -maxdepth 3 -type f -print | sort
