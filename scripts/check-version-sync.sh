#!/bin/bash
set -euo pipefail

repo_root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$repo_root"
expected_version="${1:-}"

json_version() {
  node --input-type=module -e \
    'import { readFileSync } from "node:fs"; const value = JSON.parse(readFileSync(process.argv[1], "utf8")).version; if (typeof value !== "string" || value.length === 0) process.exit(2); process.stdout.write(value);' \
    "$1"
}

package_version="$(json_version package.json)"
lock_version="$(json_version package-lock.json)"
lock_root_version="$(node --input-type=module -e \
  'import { readFileSync } from "node:fs"; const value = JSON.parse(readFileSync("package-lock.json", "utf8")).packages?.[""]?.version; if (typeof value !== "string" || value.length === 0) process.exit(2); process.stdout.write(value);')"
tauri_version="$(json_version src-tauri/tauri.conf.json)"
cargo_versions="$(cargo metadata --no-deps --format-version 1 | node --input-type=module -e \
  'let input = ""; for await (const chunk of process.stdin) input += chunk; const metadata = JSON.parse(input); const members = new Set(metadata.workspace_members); const versions = [...new Set(metadata.packages.filter((pkg) => members.has(pkg.id)).map((pkg) => pkg.version))].sort(); if (versions.length !== 1) process.exit(2); process.stdout.write(versions[0]);')"

canonical_version="$package_version"
for version_entry in \
  "package-lock.json:$lock_version" \
  "package-lock root:$lock_root_version" \
  "Tauri:$tauri_version" \
  "Cargo workspace:$cargo_versions"; do
  label="${version_entry%%:*}"
  candidate="${version_entry#*:}"
  if [[ "$candidate" != "$canonical_version" ]]; then
    echo "版本不一致：package.json=$canonical_version，$label=$candidate" >&2
    exit 1
  fi
done

if [[ -n "$expected_version" && "$canonical_version" != "$expected_version" ]]; then
  echo "发布标签 $expected_version 与源码版本 $canonical_version 不一致。" >&2
  exit 1
fi

printf 'version-sync=%s\n' "$canonical_version"
