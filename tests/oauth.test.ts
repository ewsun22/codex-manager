import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { invokeDemo } from "../src/app/demo.ts";
import {
  COMMANDS,
  type AuthProfileOperationResult,
  type AuthProfilesSnapshot,
  type CodexAccountSnapshot,
  type CodexLoginStartResult,
} from "../src/shared/contracts.ts";

test("OAuth 账户摘要只暴露展示所需的白名单字段", async () => {
  const snapshot = await invokeDemo<CodexAccountSnapshot>(COMMANDS.getCodexAccount);

  assert.deepEqual(Object.keys(snapshot).sort(), [
    "authMethod",
    "authenticated",
    "availableResetCredits",
    "cachedAt",
    "checkedAt",
    "email",
    "loginInProgress",
    "message",
    "planType",
    "rateLimits",
    "rateLimitsAvailable",
    "requiresOpenaiAuth",
    "source",
    "stale",
    "state",
  ]);
  assert.doesNotMatch(
    JSON.stringify(snapshot),
    /access[_-]?token|refresh[_-]?token|authorization|oauth\/authorize|auth\.json/i,
  );
});

test("OAuth 登录入口只返回进程状态，不返回授权链接或回调", async () => {
  const result = await invokeDemo<CodexLoginStartResult>(COMMANDS.startCodexLogin);

  assert.deepEqual(result, { started: true, loginInProgress: true });
  assert.deepEqual(Object.keys(result).sort(), ["loginInProgress", "started"]);
});

test("认证档案列表只暴露不可逆元数据，不返回秘密、路径或账户标识", async () => {
  const snapshot = await invokeDemo<AuthProfilesSnapshot>(COMMANDS.listAuthProfiles);

  assert.deepEqual(Object.keys(snapshot).sort(), [
    "activationAvailable",
    "activeProfileId",
    "activeRevision",
    "deletedRetentionDays",
    "message",
    "profiles",
    "supported",
  ]);
  assert.ok(snapshot.profiles.length > 0);
  for (const profile of snapshot.profiles) {
    assert.deepEqual(Object.keys(profile).sort(), [
      "createdAt",
      "deletedAt",
      "id",
      "isActive",
      "label",
      "updatedAt",
    ]);
  }
  assert.doesNotMatch(
    JSON.stringify(snapshot.profiles),
    /"(?:accessToken|refreshToken|authorization|authJson|path|fileName|identityFingerprint|email)"/i,
  );
});

test("认证档案写命令只返回变更状态和安全消息", async () => {
  const result = await invokeDemo<AuthProfileOperationResult>(COMMANDS.importAuthProfile, {
    label: "测试档案",
  });

  assert.deepEqual(Object.keys(result).sort(), ["changed", "message"]);
  assert.equal(result.changed, true);
  assert.doesNotMatch(JSON.stringify(result), /access[_-]?token|refresh[_-]?token|authorization|oauth\/authorize/i);
});

test("演示认证档案遵守活动、删除、恢复与切换约束", async () => {
  let snapshot = await invokeDemo<AuthProfilesSnapshot>(COMMANDS.listAuthProfiles);
  const active = snapshot.profiles.find((profile) => profile.isActive);
  const inactive = snapshot.profiles.find((profile) => !profile.isActive && profile.deletedAt === null);
  assert.ok(active);
  assert.ok(inactive);

  await assert.rejects(
    invokeDemo(COMMANDS.deleteAuthProfile, { profileId: active.id }),
    /不能删除当前生效/,
  );
  await invokeDemo(COMMANDS.deleteAuthProfile, { profileId: inactive.id });
  snapshot = await invokeDemo<AuthProfilesSnapshot>(COMMANDS.listAuthProfiles);
  assert.equal(snapshot.profiles.find((profile) => profile.id === inactive.id)?.deletedAt === null, false);
  await assert.rejects(
    invokeDemo(COMMANDS.activateAuthProfile, { profileId: inactive.id }),
    /不存在或已删除/,
  );
  await invokeDemo(COMMANDS.restoreAuthProfile, { profileId: inactive.id });
  await invokeDemo(COMMANDS.activateAuthProfile, { profileId: inactive.id });
  snapshot = await invokeDemo<AuthProfilesSnapshot>(COMMANDS.listAuthProfiles);
  assert.equal(snapshot.activeProfileId, inactive.id);
  assert.equal(snapshot.profiles.find((profile) => profile.id === inactive.id)?.isActive, true);
});

test("OAuth WebView 权限是窄命令且不获得 Shell、HTTP 或进程插件", () => {
  const capability = JSON.parse(
    readFileSync(new URL("../src-tauri/capabilities/default.json", import.meta.url), "utf8"),
  ) as { permissions: string[] };

  assert.ok(capability.permissions.includes("allow-get-codex-account"));
  assert.ok(capability.permissions.includes("allow-start-codex-login"));
  for (const permission of [
    "allow-list-auth-profiles",
    "allow-import-auth-profile",
    "allow-activate-auth-profile",
    "allow-delete-auth-profile",
    "allow-restore-auth-profile",
  ]) {
    assert.ok(capability.permissions.includes(permission));
  }
  assert.equal(
    capability.permissions.some((permission) => /^(shell|http|process|opener|dialog):/.test(permission)),
    false,
  );
});

test("原生导入命令不接受 WebView 提供的路径或认证 JSON", () => {
  const mainSource = readFileSync(new URL("../src-tauri/src/main.rs", import.meta.url), "utf8");
  const commandStart = mainSource.indexOf("async fn import_auth_profile(");
  const commandEnd = mainSource.indexOf("async fn activate_auth_profile(", commandStart);
  assert.ok(commandStart >= 0 && commandEnd > commandStart);
  const commandSource = mainSource.slice(commandStart, commandEnd);

  assert.match(commandSource, /pick_auth_file\(&app\)/);
  assert.doesNotMatch(commandSource.slice(0, commandSource.indexOf(") ->")), /path|json|bytes|content/i);
});

test("认证档案首次读取失败后不会由 effect 无限重试", () => {
  const appSource = readFileSync(new URL("../src/App.tsx", import.meta.url), "utf8");

  assert.match(appSource, /const profilesAutoRequested = useRef\(false\)/);
  assert.match(
    appSource,
    /activeTab !== "credentials" \|\| profiles \|\| profilesAutoRequested\.current/,
  );
  assert.match(appSource, /profilesAutoRequested\.current = true;\s+void refreshProfiles\(\)/);
  assert.doesNotMatch(
    appSource,
    /activeTab === "credentials" && !profiles && !profilesLoading.*refreshProfiles\(\)/,
  );
});

test("官方订阅首次读取缓存，只有显式刷新和账户变更才强制读取", () => {
  const appSource = readFileSync(new URL("../src/App.tsx", import.meta.url), "utf8");

  assert.match(appSource, /getCodexAccount, \{ refresh: force \}/);
  assert.match(appSource, /void refresh\(false, true\)/);
  assert.match(appSource, /onClick=\{\(\) => void refresh\(true\)\}/);
  assert.match(appSource, /await refresh\(true, true\)/);
  assert.match(appSource, /上次确认数据（可能已过期）/);
  assert.match(appSource, /刷新失败：/);
});
