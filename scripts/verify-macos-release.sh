#!/bin/bash
set -euo pipefail

required_vars=(ASSET_DIR ASSET_RECORDS_PATH BUILD_INPUT_PATH RELEASE_ID REPOSITORY SOURCE_SHA TOOLS_DIR VERSION)
for required_var in "${required_vars[@]}"; do
  if [ -z "${!required_var:-}" ]; then
    echo "Release verification input $required_var is unavailable." >&2
    exit 1
  fi
done
ASSET_DIR="${ASSET_DIR:?}"
ASSET_RECORDS_PATH="${ASSET_RECORDS_PATH:?}"
BUILD_INPUT_PATH="${BUILD_INPUT_PATH:?}"
TOOLS_DIR="${TOOLS_DIR:?}"

required_tools=(awk cmp codesign file find grep hdiutil jq node plutil readlink sed shasum spctl tar unzip wc xcrun)
for required_tool in "${required_tools[@]}"; do
  if ! command -v "$required_tool" >/dev/null 2>&1; then
    echo "Release verification tool is unavailable: $required_tool" >&2
    exit 1
  fi
done
test -x /usr/libexec/PlistBuddy

asset_dir="$(cd "$ASSET_DIR" && pwd)"
tools_dir="$(cd "$TOOLS_DIR" && pwd)"
build_input_path="$(cd "$(dirname "$BUILD_INPUT_PATH")" && pwd)/$(basename "$BUILD_INPUT_PATH")"
asset_records_path="$(cd "$(dirname "$ASSET_RECORDS_PATH")" && pwd)/$(basename "$ASSET_RECORDS_PATH")"
repo_root="$(cd "${BASH_SOURCE[0]%/*}/.." && pwd)"
work_root="$(mktemp -d "${RUNNER_TEMP:-/tmp}/codex-manager-release-verify.XXXXXX")"
mount_point="$work_root/dmg-mount"
mounted=false

cleanup() {
  if [ "$mounted" = true ]; then hdiutil detach "$mount_point" >/dev/null 2>&1 || true; fi
  rm -rf -- "$work_root"
}
trap cleanup EXIT

stem="codex-manager-v$VERSION"
expected_names=(
  BUILD-PROVENANCE.json
  SIGNING-EVIDENCE.json
  "SHA256SUMS-v$VERSION-signed"
  "$stem-aarch64.app.tar.gz"
  "$stem-aarch64.app.tar.gz.sig"
  "$stem-aarch64.dmg"
  "$stem-licenses.zip"
  "$stem-sbom.zip"
  latest.json
)
expected_list="$work_root/expected-names.txt"
actual_list="$work_root/actual-names.txt"
printf '%s\n' "${expected_names[@]}" | LC_ALL=C sort > "$expected_list"
find "$asset_dir" -maxdepth 1 -type f -exec basename {} \; | LC_ALL=C sort > "$actual_list"
cmp -s "$expected_list" "$actual_list"

manifest="$asset_dir/SHA256SUMS-v$VERSION-signed"
test "$(wc -l < "$manifest" | tr -d ' ')" -eq 8
if grep -Eqv '^[a-f0-9]{64}  [^/]+$' "$manifest"; then
  echo "Release checksum manifest contains an invalid or non-flat entry." >&2
  exit 1
fi
manifest_names="$work_root/manifest-names.txt"
grep -v -Fx "$(basename "$manifest")" "$expected_list" > "$work_root/expected-manifest-names.txt"
sed -E 's/^[a-f0-9]{64}  //' "$manifest" | LC_ALL=C sort > "$manifest_names"
cmp -s "$work_root/expected-manifest-names.txt" "$manifest_names"
(cd "$asset_dir" && shasum -a 256 -c "$(basename "$manifest")")

REPOSITORY="$REPOSITORY" VERSION="$VERSION" SOURCE_SHA="$SOURCE_SHA" \
  node "$repo_root/scripts/release-metadata.mjs" validate-metadata \
  --asset-dir "$asset_dir" --asset-records "$asset_records_path" \
  --build-input "$build_input_path" --release-id "$RELEASE_ID"

tarball="$asset_dir/$stem-aarch64.app.tar.gz"
signature="$tarball.sig"
dmg="$asset_dir/$stem-aarch64.dmg"
node "$repo_root/scripts/verify-updater-signature.mjs" "$tarball" "$signature" "$tools_dir/updater.pub"

unzip -t "$asset_dir/$stem-sbom.zip"
unzip -t "$asset_dir/$stem-licenses.zip"
sbom_extract="$work_root/sbom"
licenses_extract="$work_root/licenses"
mkdir -p "$sbom_extract" "$licenses_extract"
unzip -q "$asset_dir/$stem-sbom.zip" -d "$sbom_extract"
unzip -q "$asset_dir/$stem-licenses.zip" -d "$licenses_extract"
(cd "$sbom_extract" && find artifacts/release/sbom -mindepth 1 -maxdepth 1 -print | LC_ALL=C sort) > "$work_root/sbom-files.txt"
cat > "$work_root/expected-sbom-files.txt" <<'EOF'
artifacts/release/sbom/SHA256SUMS
artifacts/release/sbom/cargo-core.cdx.json
artifacts/release/sbom/cargo-desktop.cdx.json
artifacts/release/sbom/cargo-storage.cdx.json
artifacts/release/sbom/npm.cdx.json
EOF
cmp -s "$work_root/expected-sbom-files.txt" "$work_root/sbom-files.txt"
(cd "$licenses_extract" && find artifacts/release/licenses -mindepth 1 -maxdepth 1 -print | LC_ALL=C sort) > "$work_root/license-files.txt"
cat > "$work_root/expected-license-files.txt" <<'EOF'
artifacts/release/licenses/THIRD_PARTY_NOTICES.md
artifacts/release/licenses/cargo-licenses.json
artifacts/release/licenses/npm-dependencies.json
EOF
cmp -s "$work_root/expected-license-files.txt" "$work_root/license-files.txt"
test -s "$licenses_extract/artifacts/release/licenses/THIRD_PARTY_NOTICES.md"
(cd "$sbom_extract" && shasum -a 256 -c artifacts/release/sbom/SHA256SUMS)
while IFS= read -r json_file; do jq -e . "$json_file" >/dev/null; done < <(find "$sbom_extract" "$licenses_extract" -type f -name '*.json' -print)

signing_evidence="$asset_dir/SIGNING-EVIDENCE.json"
expected_team="$(jq -er .apple.teamId "$signing_evidence")"
expected_authority_sha="$(jq -er .apple.authoritySha256 "$signing_evidence")"
expected_leaf_sha="$(jq -er .apple.leafCertificateSha256 "$signing_evidence")"
expected_app_cdhash="$(jq -er .apple.app.cdHash "$signing_evidence")"
expected_dmg_cdhash="$(jq -er .apple.dmg.cdHash "$signing_evidence")"
expected_updater_key_sha="$(jq -er .updater.publicKeySha256 "$signing_evidence")"
test "$(shasum -a 256 "$tools_dir/updater.pub" | awk '{print $1}')" = "$expected_updater_key_sha"

verify_leaf_certificate() {
  local candidate="$1"
  local label="$2"
  local cert_dir="$work_root/cert-$label"
  mkdir -p "$cert_dir"
  (cd "$cert_dir" && codesign --display --extract-certificates "$candidate" >/dev/null 2>&1)
  test -s "$cert_dir/codesign0"
  test "$(shasum -a 256 "$cert_dir/codesign0" | awk '{print $1}')" = "$expected_leaf_sha"
}

verify_app() {
  local candidate="$1"
  local label="$2"
  codesign --verify --deep --strict --verbose=2 "$candidate"
  local info authority authority_sha entitlements executable gatekeeper
  info="$(codesign --display --verbose=4 "$candidate" 2>&1)"
  authority="$(awk -F= '/^Authority=Developer ID Application:/{print substr($0, index($0,"=") + 1); exit}' <<< "$info")"
  test -n "$authority"
  authority_sha="$(printf '%s' "$authority" | shasum -a 256 | awk '{print $1}')"
  test "$authority_sha" = "$expected_authority_sha"
  grep -Fqx "TeamIdentifier=$expected_team" <<< "$info"
  grep -Eq '^CodeDirectory .* flags=.*runtime' <<< "$info"
  grep -Eq '^Timestamp=' <<< "$info"
  test "$(codesign --display --verbose=4 "$candidate" 2>&1 | awk -F= '/^CDHash=/{print $2; exit}')" = "$expected_app_cdhash"
  test "$(/usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' "$candidate/Contents/Info.plist")" = "cc.codex.manager"
  test "$(/usr/libexec/PlistBuddy -c 'Print :CFBundleShortVersionString' "$candidate/Contents/Info.plist")" = "$VERSION"
  test "$(/usr/libexec/PlistBuddy -c 'Print :CFBundleVersion' "$candidate/Contents/Info.plist")" = "$VERSION"
  executable="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleExecutable' "$candidate/Contents/Info.plist")"
  [[ "$(file "$candidate/Contents/MacOS/$executable")" == *"Mach-O 64-bit executable arm64"* ]]
  entitlements="$work_root/$label-entitlements.plist"
  codesign --display --entitlements :- "$candidate" > "$entitlements" 2>/dev/null
  if [ -s "$entitlements" ]; then plutil -lint "$entitlements" >/dev/null; fi
  if grep -Eq 'com\.apple\.security\.(get-task-allow|cs\.disable-library-validation|cs\.allow-unsigned-executable-memory|cs\.allow-dyld-environment-variables)' "$entitlements"; then
    echo "A remotely downloaded app contains a forbidden release entitlement." >&2
    exit 1
  fi
  verify_leaf_certificate "$candidate" "$label"
  xcrun stapler validate "$candidate"
  gatekeeper="$(spctl --assess --type execute --verbose=4 "$candidate" 2>&1)"
  grep -Fq 'source=Notarized Developer ID' <<< "$gatekeeper"
}

updater_extract="$work_root/updater"
mkdir -p "$updater_extract"
tar -xzf "$tarball" -C "$updater_extract"
test "$(find "$updater_extract" -mindepth 1 -maxdepth 1 -print | wc -l | tr -d ' ')" -eq 1
updater_app="$(find "$updater_extract" -maxdepth 1 -type d -name '*.app' -print -quit)"
test -d "$updater_app"
verify_app "$updater_app" updater

codesign --verify --strict --verbose=2 "$dmg"
dmg_info="$(codesign --display --verbose=4 "$dmg" 2>&1)"
dmg_authority="$(awk -F= '/^Authority=Developer ID Application:/{print substr($0, index($0,"=") + 1); exit}' <<< "$dmg_info")"
test -n "$dmg_authority"
test "$(printf '%s' "$dmg_authority" | shasum -a 256 | awk '{print $1}')" = "$expected_authority_sha"
grep -Fqx "TeamIdentifier=$expected_team" <<< "$dmg_info"
grep -Eq '^Timestamp=' <<< "$dmg_info"
test "$(awk -F= '/^CDHash=/{print $2; exit}' <<< "$dmg_info")" = "$expected_dmg_cdhash"
verify_leaf_certificate "$dmg" dmg
xcrun stapler validate "$dmg"
hdiutil verify "$dmg"
dmg_gatekeeper="$(spctl --assess --type open --context context:primary-signature --verbose=4 "$dmg" 2>&1)"
grep -Fq 'source=Notarized Developer ID' <<< "$dmg_gatekeeper"

mkdir -p "$mount_point"
hdiutil attach "$dmg" -readonly -nobrowse -noautoopen -mountpoint "$mount_point" >/dev/null
mounted=true
test "$(find "$mount_point" -mindepth 1 -maxdepth 1 -print | wc -l | tr -d ' ')" -eq 2
test -L "$mount_point/Applications"
test "$(readlink "$mount_point/Applications")" = "/Applications"
dmg_app="$(find "$mount_point" -maxdepth 1 -type d -name '*.app' -print -quit)"
test -d "$dmg_app"
verify_app "$dmg_app" dmg-app
test "$(codesign --display --verbose=4 "$updater_app" 2>&1 | awk -F= '/^CDHash=/{print $2; exit}')" = \
  "$(codesign --display --verbose=4 "$dmg_app" 2>&1 | awk -F= '/^CDHash=/{print $2; exit}')"
hdiutil detach "$mount_point"
mounted=false

echo "macos_release_payload_verification=ok"
