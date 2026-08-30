import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { invokeDemo } from "../src/app/demo.ts";
import {
  COMMANDS,
  type CodexConfigSnapshot,
  type CodexGatewayStatus,
  type CodexProviderProfile,
  type ProxyAuthProfileList,
} from "../src/shared/contracts.ts";

const providerKeys = [
  "baseUrl",
  "createdAt",
  "hasApiKey",
  "id",
  "model",
  "name",
  "reasoningEffort",
  "updatedAt",
];

test("供应商列表仅暴露元数据，API Key 保持只写", async () => {
  const providers = await invokeDemo<CodexProviderProfile[]>(COMMANDS.listCodexProviders);
  assert.ok(providers.length > 0);
  for (const provider of providers) {
    assert.deepEqual(Object.keys(provider).sort(), providerKeys);
  }
  const serializedValues = providers.flatMap((provider) => Object.entries(provider))
    .filter(([key]) => key !== "hasApiKey")
    .map(([, value]) => value)
    .join(" ");
  assert.doesNotMatch(
    serializedValues,
    /api[_-]?key|authorization|bearer|experimental_bearer_token|access[_-]?token|refresh[_-]?token/i,
  );
});

test("保存供应商不会把提交的秘密回传给 WebView", async () => {
  const secret = "fixture-write-only-value";
  const provider = await invokeDemo<CodexProviderProfile>(COMMANDS.saveCodexProvider, {
    input: {
      name: "安全测试供应商",
      baseUrl: "https://api.example.test/v1",
      model: "gpt-test",
      reasoningEffort: "medium",
      apiKey: secret,
    },
  });

  assert.deepEqual(Object.keys(provider).sort(), providerKeys);
  assert.equal(provider.hasApiKey, true);
  assert.doesNotMatch(JSON.stringify(provider), new RegExp(secret));
});

test("网关普通状态不包含本机 bearer，WebView 没有凭据揭示入口", async () => {
  const status = await invokeDemo<CodexGatewayStatus>(COMMANDS.getCodexGatewayStatus);
  assert.deepEqual(Object.keys(status).sort(), [
    "coreSource",
    "coreVersion",
    "endpoint",
    "failed",
    "inFlight",
    "installed",
    "installing",
    "lastError",
    "latestVersion",
    "port",
    "processId",
    "providerId",
    "providerName",
    "requests",
    "source",
    "startedAt",
    "state",
    "updateAvailable",
  ]);
  assert.doesNotMatch(JSON.stringify(status), /authorization|bearer|token|secret|api[_-]?key/i);

  const [provider] = await invokeDemo<CodexProviderProfile[]>(COMMANDS.listCodexProviders);
  assert.ok(provider);
  const running = await invokeDemo<CodexGatewayStatus>(COMMANDS.startCodexGateway, {
    source: "external-provider",
    providerId: provider.id,
  });
  assert.equal(running.source, "external-provider");
  assert.doesNotMatch(JSON.stringify(running), /authorization|bearer|token|secret|api[_-]?key/i);
  assert.equal("revealCodexGatewaySetup" in COMMANDS, false);
  await invokeDemo<CodexGatewayStatus>(COMMANDS.stopCodexGateway);
});

test("网关 WebView 权限只开放显式命令且不引入通用网络或进程权限", () => {
  const capability = JSON.parse(
    readFileSync(new URL("../src-tauri/capabilities/default.json", import.meta.url), "utf8"),
  ) as { permissions: string[] };

  for (const command of [
    "list-codex-providers",
    "save-codex-provider",
    "delete-codex-provider",
    "get-codex-gateway-status",
    "update-codex-gateway-port",
    "start-codex-gateway",
    "stop-codex-gateway",
    "check-latest-cliproxy-core",
    "install-latest-cliproxy-core",
    "get-codex-config-snapshot",
    "save-codex-config-profile",
    "delete-codex-config-profile",
    "preview-codex-config-profile",
    "apply-codex-config-profile",
    "restore-codex-config",
    "list-proxy-auth-profiles",
    "import-proxy-auth-profile",
    "set-proxy-auth-profile-enabled",
    "delete-proxy-auth-profile",
    "restore-proxy-auth-profile",
  ]) {
    assert.ok(capability.permissions.includes(`allow-${command}`));
  }
  assert.equal(
    capability.permissions.some((permission) => /^(shell|http|process|opener|dialog):/.test(permission)),
    false,
  );
});

test("两个产品页拆分本地代理与 Codex 配置的信任边界", () => {
  const proxySource = readFileSync(
    new URL("../src/app/codex-gateway.tsx", import.meta.url),
    "utf8",
  );
  const configSource = readFileSync(new URL("../src/app/codex-config.tsx", import.meta.url), "utf8");
  const shell = readFileSync(new URL("../src/App.tsx", import.meta.url), "utf8");

  assert.match(proxySource, /<h1>本地代理<\/h1>/);
  assert.match(proxySource, /CLIProxyAPI OAuth 档案/);
  assert.match(proxySource, /非上游联网验真/);
  assert.match(proxySource, /COMMANDS\.checkLatestCliproxyCore/);
  assert.match(proxySource, /COMMANDS\.installLatestCliproxyCore/);
  assert.doesNotMatch(proxySource, /revealCodexGatewaySetup|experimental_bearer_token/);
  assert.match(configSource, /<h1>Codex 配置<\/h1>/);
  assert.match(configSource, /官方直连/);
  assert.match(configSource, /本地 CLIProxyAPI/);
  assert.match(configSource, /外部 Responses 兼容代理/);
  assert.match(configSource, /打开官方订阅/);
  assert.match(shell, /id: "config", label: "Codex 配置"/);
  assert.match(shell, /id: "gateway", label: "本地代理"/);
  assert.match(shell, /onOpenOfficialSubscription=\{\(\) => setView\("oauth"\)\}/);
});

test("代理 OAuth DTO 与 Codex 配置快照不回传 token 或原始认证文件", async () => {
  const auth = await invokeDemo<ProxyAuthProfileList>(COMMANDS.listProxyAuthProfiles, { includeDeleted: true });
  assert.ok(auth.profiles.length > 0);
  assert.deepEqual(Object.keys(auth.profiles[0]!).sort(), [
    "createdAt", "enabled", "errorCode", "id", "label", "lastCheckpointAt", "provider", "state", "updatedAt",
  ]);
  assert.doesNotMatch(JSON.stringify(auth), /access[_-]?token|refresh[_-]?token|authorization|bearer/i);

  const config = await invokeDemo<CodexConfigSnapshot>(COMMANDS.getCodexConfigSnapshot);
  assert.ok(config.profiles.some((profile) => profile.kind === "official-direct"));
  assert.doesNotMatch(JSON.stringify(config), /access[_-]?token|refresh[_-]?token|authorization|bearer/i);
});

test("首页代理快捷控制仅复制 endpoint，支持可访问的启停状态", () => {
  const source = readFileSync(new URL("../src/app/codex-gateway.tsx", import.meta.url), "utf8");
  const quickControl = source.slice(source.indexOf("export function CodexGatewayQuickControl"), source.indexOf("export function CodexGatewayView"));

  assert.match(quickControl, /CLIProxyAPI/);
  assert.match(quickControl, /\["OpenAI", endpoint\]/);
  assert.match(quickControl, /\["Claude", proxyOrigin\]/);
  assert.match(quickControl, /\["Gemini", proxyOrigin\]/);
  assert.match(quickControl, /navigator\.clipboard\.writeText\(value\)/);
  assert.doesNotMatch(quickControl, /clipboard\.writeText\(.*(?:bearer|apiKey|configSnippet)/i);
  assert.match(quickControl, /role="switch"/);
  assert.match(quickControl, /aria-checked=\{running\}/);
  assert.match(quickControl, /COMMANDS\.installLatestCliproxyCore/);
  assert.match(quickControl, /aria-busy=\{busy !== null\}/);
  assert.match(quickControl, /aria-live="polite"/);
  assert.doesNotMatch(source, /全局提示词|插件与市场|会话管理/);
});

test("CLIProxyAPI 内核版本与桌面应用独立检查和更新", async () => {
  const checked = await invokeDemo<CodexGatewayStatus>(COMMANDS.checkLatestCliproxyCore);
  assert.equal(checked.latestVersion, "7.2.145");
  assert.equal(checked.coreSource, "GitHub Release");

  const installed = await invokeDemo<CodexGatewayStatus>(COMMANDS.installLatestCliproxyCore);
  assert.equal(installed.installed, true);
  assert.equal(installed.coreVersion, installed.latestVersion);
  assert.equal(installed.updateAvailable, false);
});

test("原生反代只启动受限 CLIProxyAPI sidecar，不保留旧内嵌转发器", () => {
  const source = readFileSync(
    new URL("../src-tauri/src/provider_gateway.rs", import.meta.url),
    "utf8",
  );

  for (const boundary of [
    "host: \\\"127.0.0.1\\\"",
    "commercial-mode: true",
    "request-log: false",
    "usage-statistics-enabled: false",
    "disable-control-panel: true",
    "disable-auto-update-panel: true",
    ".arg(\"-local-model\")",
    ".env_clear()",
  ]) {
    assert.match(source, new RegExp(boundary.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")));
  }
  assert.doesNotMatch(source, /axum::serve|fn forward\(|struct Counters|start_with_secret/);
});
