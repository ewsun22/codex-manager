import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { invokeDemo } from "../src/app/demo.ts";
import { COMMANDS, type AppUpdateStatus, type UpdateInstallResult } from "../src/shared/contracts.ts";

test("更新检查只返回可展示的版本元数据", async () => {
  const status = await invokeDemo<AppUpdateStatus>(COMMANDS.checkForUpdate);

  assert.equal(status.currentVersion, "0.1.0");
  assert.equal(status.available, true);
  assert.equal(status.version, "0.2.1");
  assert.deepEqual(Object.keys(status).sort(), ["available", "currentVersion", "date", "notes", "version"]);
});

test("安装只接受上一次检查的预期版本", async () => {
  await assert.rejects(
    invokeDemo(COMMANDS.installPendingUpdate, { expectedVersion: "9.9.9" }),
    /更新版本已变化/,
  );
  const result = await invokeDemo<UpdateInstallResult>(COMMANDS.installPendingUpdate, { expectedVersion: "0.2.1" });
  assert.equal(result.accepted, true);
});

test("前端只获得受限更新命令，不获得插件权限", () => {
  const capability = JSON.parse(readFileSync(new URL("../src-tauri/capabilities/default.json", import.meta.url), "utf8")) as { permissions: string[] };

  assert.ok(capability.permissions.includes("allow-check-for-update"));
  assert.ok(capability.permissions.includes("allow-install-pending-update"));
  assert.equal(capability.permissions.some((permission) => /^(updater|dialog|http|process):/.test(permission)), false);
});

test("更新源与公钥固定，只有发布配置生成更新产物", () => {
  const config = JSON.parse(readFileSync(new URL("../src-tauri/tauri.conf.json", import.meta.url), "utf8"));
  const releaseConfig = JSON.parse(readFileSync(new URL("../src-tauri/tauri.updater.conf.json", import.meta.url), "utf8"));

  assert.deepEqual(config.plugins.updater.endpoints, ["https://github.com/ewsun22/codex-manager/releases/latest/download/latest.json"]);
  assert.ok(config.plugins.updater.pubkey.length > 40);
  assert.notEqual(config.bundle.createUpdaterArtifacts, true);
  assert.equal(releaseConfig.bundle.createUpdaterArtifacts, true);
});

test("发布工作流以可执行状态机隔离签名、恢复和草稿远端复验", () => {
  const workflow = readFileSync(new URL("../.github/workflows/release.yml", import.meta.url), "utf8");
  const releaseControl = readFileSync(new URL("../scripts/release-github.mjs", import.meta.url), "utf8");
  const releaseProtocol = readFileSync(new URL("../scripts/release-state.mjs", import.meta.url), "utf8");
  const macosVerifier = readFileSync(new URL("../scripts/verify-macos-release.sh", import.meta.url), "utf8");
  const lock = JSON.parse(readFileSync(new URL("../package-lock.json", import.meta.url), "utf8"));

  assert.equal(workflow.match(/actions\/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1/g)?.length, 4);
  assert.equal(workflow.match(/actions\/setup-node@820762786026740c76f36085b0efc47a31fe5020/g)?.length, 4);
  assert.equal(workflow.match(/persist-credentials: false/g)?.length, 4);
  assert.doesNotMatch(workflow, /git fetch/);
  assert.doesNotMatch(workflow, /tauri-action/);
  assert.doesNotMatch(workflow, /--clobber/);
  assert.doesNotMatch(workflow, /releases\/tags/);
  assert.doesNotMatch(workflow, /gh release create/);
  assert.doesNotMatch(workflow, /gh release upload/);
  assert.doesNotMatch(workflow, /\.browser_download_url/);
  assert.doesNotMatch(workflow, /untagged-/);
  assert.doesNotMatch(workflow, /for attempt in 1 2 3 4 5 6 7 8 9 10/);
  assert.doesNotMatch(workflow, /KEYCHAIN_PASSWORD/);
  assert.equal(workflow.match(/TAURI_SIGNING_PRIVATE_KEY: \$\{\{ secrets\.TAURI_SIGNING_PRIVATE_KEY \}\}/g)?.length, 1);
  assert.equal(
    workflow.match(/TAURI_SIGNING_PRIVATE_KEY_PASSWORD: \$\{\{ secrets\.TAURI_SIGNING_PRIVATE_KEY_PASSWORD \}\}/g)?.length,
    1,
  );
  assert.match(workflow, /resume_run_id:/);
  assert.match(workflow, /resume_release_id:/);
  assert.match(workflow, /create-or-resolve-draft/);
  assert.equal(workflow.match(/scripts\/release-github\.mjs sync-assets/g)?.length, 2);
  assert.match(workflow, /scripts\/release-github\.mjs download-assets/);
  assert.match(workflow, /scripts\/release-github\.mjs verify-state/);
  assert.match(workflow, /scripts\/release-metadata\.mjs validate-intent/);
  assert.match(workflow, /scripts\/release-metadata\.mjs make-provenance/);
  assert.match(workflow, /sign:[\s\S]+environment: release[\s\S]+permissions:\n      contents: read/);
  assert.match(workflow, /draft:[\s\S]+permissions:\n      contents: write/);
  assert.match(workflow, /npm run tauri -- build --target aarch64-apple-darwin --bundles app --no-sign/);
  assert.match(workflow, new RegExp(lock.packages["node_modules/@tauri-apps/cli"].integrity.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")));
  assert.match(workflow, new RegExp(lock.packages["node_modules/@tauri-apps/cli-darwin-arm64"].integrity.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")));
  assert.doesNotMatch(workflow, /target\/release\/examples\/verify_updater_signature/);
  assert.equal(workflow.match(/node scripts\/verify-updater-signature\.mjs/g)?.length, 2);
  assert.match(macosVerifier, /node "\$repo_root\/scripts\/verify-updater-signature\.mjs"/);
  assert.match(workflow, /codesign --deep --force --options runtime --timestamp/);
  assert.equal(workflow.match(/\^CodeDirectory \.\* flags=\.\*runtime/g)?.length, 2);
  assert.match(macosVerifier, /\^CodeDirectory \.\* flags=\.\*runtime/);
  assert.match(workflow, /hdiutil attach "\$dmg"[^\n]+-mountpoint "\$mount_point" >\/dev\/null/);
  assert.match(macosVerifier, /hdiutil attach "\$dmg"[^\n]+-mountpoint "\$mount_point" >\/dev\/null/);
  assert.match(workflow, /hdiutil detach "\$mount_point"/);
  assert.doesNotMatch(workflow, /device="\$\(hdiutil attach/);
  assert.match(
    workflow,
    /xcrun notarytool submit "\$app_archive"[\s\S]+select\(\.status == "Accepted"\)[\s\S]+xcrun stapler validate "\$app_bundle"/,
  );
  assert.match(
    workflow,
    /node "\$signer_root\/cli\/tauri\.js" signer sign[\s\S]+unset TAURI_SIGNING_PRIVATE_KEY TAURI_SIGNING_PRIVATE_KEY_PASSWORD[\s\S]+verify-updater-signature\.mjs/,
  );
  assert.match(workflow, /xcrun notarytool submit "\$dmg"[\s\S]+select\(\.status == "Accepted"\)[\s\S]+xcrun stapler validate "\$dmg"/);
  assert.match(workflow, /actions\/attest@1e69f48acb82d1966a394da916b4c1698aa569d6/);
  assert.match(workflow, /SHA256SUMS-v\$VERSION-signed/);
  assert.match(workflow, /cmp -s "\$local_asset" "\$remote\//);
  assert.match(releaseProtocol, /DEFAULT_DEADLINE_MS = 5 \* 60 \* 1000/);
  assert.match(releaseProtocol, /releases\/download\/\$\{canonicalTag\(version\)\}/);
  assert.match(releaseControl, /--signer-digest/);
  assert.match(releaseControl, /--deny-self-hosted-runners/);
  assert.match(releaseControl, /downloadWithRetry/);
});

test("只读发布复验覆盖 draft 与 published 且没有 Release 写权限", () => {
  const workflow = readFileSync(new URL("../.github/workflows/verify-release.yml", import.meta.url), "utf8");

  assert.match(workflow, /options:\n          - draft\n          - published/);
  assert.match(workflow, /permissions:\n  contents: read\n  attestations: read/);
  assert.doesNotMatch(workflow, /contents: write/);
  assert.doesNotMatch(workflow, /id-token: write/);
  assert.doesNotMatch(workflow, /create-or-resolve-draft|sync-assets|gh release|--method POST/);
  assert.match(workflow, /scripts\/release-github\.mjs download-release/);
  assert.match(workflow, /scripts\/release-github\.mjs verify-state/);
  assert.match(workflow, /scripts\/verify-macos-release\.sh/);
  assert.match(workflow, /scripts\/release-github\.mjs verify-attestations/);
  assert.match(workflow, /scripts\/release-metadata\.mjs make-verification/);
});
