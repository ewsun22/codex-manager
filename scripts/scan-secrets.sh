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

secret_pattern='(-----BEGIN (RSA |EC |OPENSSH )?PRIVATE KEY-----|untrusted comment: minisign encrypted secret key|AKIA[0-9A-Z]{16}|gh[pousr]_[A-Za-z0-9_]{30,}|sk-[A-Za-z0-9_-]{20,}|Authorization:[[:space:]]*Bearer[[:space:]]+[A-Za-z0-9._~-]{12,}|(TAURI_SIGNING_PRIVATE_KEY|APPLE_PASSWORD|APPLE_CERTIFICATE_PASSWORD)[[:space:]]*=[[:space:]]*[A-Za-z0-9+/=_-]{8,})'
matches="$(xargs -0 rg -l -I --pcre2 "$secret_pattern" < "$candidate_list" | rg -v '^scripts/scan-secrets\.sh$' || true)"
if [[ -n "$matches" ]]; then
  echo "检测到疑似密钥，仅列出文件名：" >&2
  echo "$matches" >&2
  exit 1
fi

certificate_files="$(tr '\0' '\n' < "$candidate_list" | rg -i '(\.p12|\.pfx|\.cer|\.crt|\.key|\.mobileprovision|/AuthKey_[^/]+\.p8)$' || true)"
if [[ -n "$certificate_files" ]]; then
  echo "检测到禁止进入仓库的证书或私钥文件，仅列出文件名：" >&2
  echo "$certificate_files" >&2
  exit 1
fi

echo "未检测到已知高风险密钥格式。"
