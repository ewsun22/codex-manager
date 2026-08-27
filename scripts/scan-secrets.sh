#!/bin/bash
set -euo pipefail

repo_root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$repo_root"

candidate_list="$(mktemp)"
trap 'rm -f "$candidate_list"' EXIT

git ls-files -co --exclude-standard -z > "$candidate_list"
if [[ ! -s "$candidate_list" ]]; then
  echo "没有可扫描的仓库文件。"
  exit 0
fi

secret_pattern='(-----BEGIN (RSA |EC |OPENSSH )?PRIVATE KEY-----|AKIA[0-9A-Z]{16}|gh[pousr]_[A-Za-z0-9_]{30,}|sk-[A-Za-z0-9_-]{20,}|Authorization:[[:space:]]*Bearer[[:space:]]+[A-Za-z0-9._~-]{12,})'
matches="$(xargs -0 rg -l -I --pcre2 "$secret_pattern" < "$candidate_list" || true)"
if [[ -n "$matches" ]]; then
  echo "检测到疑似密钥，仅列出文件名：" >&2
  echo "$matches" >&2
  exit 1
fi

echo "未检测到已知高风险密钥格式。"
