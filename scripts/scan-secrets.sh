#!/bin/bash
set -euo pipefail

script_dir="${BASH_SOURCE[0]%/*}"
if ! command -v node >/dev/null 2>&1; then
  echo "密钥扫描未完成：缺少 Node.js。" >&2
  exit 2
fi

exec node "$script_dir/scan-secrets.mjs" "$@"
