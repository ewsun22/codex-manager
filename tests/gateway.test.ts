import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { invokeDemo } from "../src/app/demo.ts";
import {
  COMMANDS,
  type CodexGatewaySetup,
  type CodexGatewayStatus,
  type CodexProviderProfile,
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

test("网关普通状态不包含本机 bearer，敏感配置只能显式揭示", async () => {
  const status = await invokeDemo<CodexGatewayStatus>(COMMANDS.getCodexGatewayStatus);
  assert.deepEqual(Object.keys(status).sort(), [
    "endpoint",
    "failed",
    "inFlight",
    "lastError",
    "port",
    "providerId",
    "providerName",
    "requests",
    "startedAt",
    "state",
  ]);
  assert.doesNotMatch(JSON.stringify(status), /authorization|bearer|token|secret|api[_-]?key/i);

  const [provider] = await invokeDemo<CodexProviderProfile[]>(COMMANDS.listCodexProviders);
  assert.ok(provider);
  await invokeDemo<CodexGatewayStatus>(COMMANDS.startCodexGateway, { providerId: provider.id });
  const setup = await invokeDemo<CodexGatewaySetup>(COMMANDS.revealCodexGatewaySetup);
  assert.deepEqual(Object.keys(setup).sort(), ["configSnippet", "endpoint", "port"]);
  assert.match(setup.configSnippet, /experimental_bearer_token/);
  assert.match(setup.configSnippet, /127\.0\.0\.1/);
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
    "reveal-codex-gateway-setup",
  ]) {
    assert.ok(capability.permissions.includes(`allow-${command}`));
  }
  assert.equal(
    capability.permissions.some((permission) => /^(shell|http|process|opener|dialog):/.test(permission)),
    false,
  );
});
