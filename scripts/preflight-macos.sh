#!/bin/bash
set -euo pipefail
echo "Codex Manager macOS release preflight (read-only)"
printf 'OS: '; sw_vers -productVersion 2>/dev/null || echo unavailable
printf 'Architecture: '; uname -m
for tool in node npm cargo rustc codesign xcrun; do
  if command -v "$tool" >/dev/null 2>&1; then printf '%-12s %s\n' "$tool" "$($tool --version 2>/dev/null | head -n 1)"; else printf '%-12s unavailable\n' "$tool"; fi
done
echo "Code signing identities (no secrets are printed):"
security find-identity -v -p codesigning 2>/dev/null || echo "无法读取 signing identity"
echo "Notarization tools:"
xcrun --find notarytool 2>/dev/null || echo "notarytool unavailable"
xcrun --find stapler 2>/dev/null || echo "stapler unavailable"
echo "Credentials are intentionally not inspected. Signing/notarization remains opt-in."
