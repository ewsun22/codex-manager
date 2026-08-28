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

test("发布工作流隔离构建、签名与草稿权限，并远端复验完整签名链", () => {
  const workflow = readFileSync(new URL("../.github/workflows/release.yml", import.meta.url), "utf8");

  assert.equal(workflow.match(/persist-credentials: false/g)?.length, 2);
  assert.doesNotMatch(workflow, /git fetch/);
  assert.doesNotMatch(workflow, /tauri-action/);
  assert.doesNotMatch(workflow, /--clobber/);
  assert.doesNotMatch(workflow, /KEYCHAIN_PASSWORD/);
  assert.equal(workflow.match(/TAURI_SIGNING_PRIVATE_KEY: \$\{\{ secrets\.TAURI_SIGNING_PRIVATE_KEY \}\}/g)?.length, 1);
  assert.equal(
    workflow.match(/TAURI_SIGNING_PRIVATE_KEY_PASSWORD: \$\{\{ secrets\.TAURI_SIGNING_PRIVATE_KEY_PASSWORD \}\}/g)?.length,
    1,
  );
  assert.match(workflow, /gh api "repos\/\$REPOSITORY\/git\/ref\/heads\/main"/);
  assert.match(workflow, /sign:[\s\S]+environment: release[\s\S]+permissions:\n      contents: read/);
  assert.match(workflow, /draft:[\s\S]+permissions:\n      contents: write/);
  assert.match(workflow, /npm run tauri -- build --target aarch64-apple-darwin --bundles app --no-sign/);
  assert.match(workflow, /sha512-R8xGtMpwyetawSqm9kYOuMmEqkhUbvcUy8n0aNXIxollKBLESUu5f4Fx\+64hgASYm1H\+jSWq6jCW6zqTnH6hqQ==/);
  assert.match(workflow, /--example verify_updater_signature/);
  assert.match(workflow, /codesign --deep --force --options runtime --timestamp/);
  assert.match(
    workflow,
    /xcrun notarytool submit "\$app_archive"[\s\S]+select\(\.status == "Accepted"\)[\s\S]+xcrun stapler validate "\$app_bundle"/,
  );
  assert.match(
    workflow,
    /node "\$signer_root\/cli\/tauri\.js" signer sign[\s\S]+unset TAURI_SIGNING_PRIVATE_KEY TAURI_SIGNING_PRIVATE_KEY_PASSWORD[\s\S]+verify_updater_signature/,
  );
  assert.match(workflow, /xcrun notarytool submit "\$dmg"[\s\S]+select\(\.status == "Accepted"\)[\s\S]+xcrun stapler validate "\$dmg"/);
  assert.match(workflow, /\.browser_download_url/);
  assert.match(workflow, /SHA256SUMS-v\$VERSION-signed/);
  assert.match(workflow, /gh attestation verify/);
  assert.match(workflow, /cmp -s "\$local_asset" "\$remote\//);
});
