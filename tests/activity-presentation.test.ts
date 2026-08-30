import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { test } from "node:test";
import { invokeDemo } from "../src/app/demo.ts";
import { formatUsd, metricProvenanceLabel } from "../src/app/format.ts";
import { COMMANDS, type ActivityPage } from "../src/shared/contracts.ts";

test("演示活动按任务和模型交互语义分别返回", async () => {
  const [turns, modelCalls] = await Promise.all([
    invokeDemo<ActivityPage>(COMMANDS.listActivity, { query: { limit: 100, view: "turns" } }),
    invokeDemo<ActivityPage>(COMMANDS.listActivity, { query: { limit: 100, view: "modelCalls" } }),
  ]);

  assert.ok(turns.items.length > 0);
  assert.ok(modelCalls.items.length > 0);
  assert.ok(turns.items.every((item) => item.activityKind === "turn"));
  assert.ok(modelCalls.items.every((item) => item.activityKind !== "turn"));
  assert.ok(modelCalls.items.some((item) => item.activityKind === "modelCall" && item.result === "accounted"));
  assert.ok(modelCalls.items.some((item) => item.activityKind === "otelRequest" && item.timingScope === "request"));
});

test("字段来源以人话形式保留在详情中", () => {
  assert.equal(metricProvenanceLabel({ source: "rollout", confidence: "high" }), "Codex 本地记录 · 高可信");
  assert.equal(metricProvenanceLabel({ source: "unavailable", confidence: "unavailable" }), "此来源未提供 · 无法评估");
});

test("首页等价估算固定显示两位小数，明细格式仍允许四位小数", async () => {
  assert.equal(formatUsd(10418.2603, 2), "US$10,418.26");
  assert.equal(formatUsd(0.0001), "US$0.0001");

  const appSource = await readFile(new URL("../src/App.tsx", import.meta.url), "utf8");
  const gatewaySource = await readFile(new URL("../src/app/codex-gateway.tsx", import.meta.url), "utf8");
  assert.match(appSource, /formatUsd\(summary\?\.equivalentCostUsd, 2\)/);
  assert.match(appSource, /payload\.activity\.items\.slice\(0, 1\)/);
  assert.match(gatewaySource, /代理 API 接入/);
  assert.match(appSource, /CLI Schema 兼容性/);
  assert.match(appSource, /buildInfo/);
});

test("活动界面具备语义切换、说明与上下两处分页控件", async () => {
  const appSource = await readFile(new URL("../src/App.tsx", import.meta.url), "utf8");
  assert.match(appSource, /role="tablist" aria-label="活动记录视图"/);
  assert.match(appSource, /event\.key === "ArrowRight" \|\| event\.key === "ArrowLeft"/);
  assert.match(appSource, /tabIndex=\{\(query\.view \?\? "turns"\) === "turns" \? 0 : -1\}/);
  assert.match(appSource, /数据说明/);
  assert.match(appSource, /showProvenance/);
  assert.equal((appSource.match(/<PaginationControls /g) ?? []).length, 2);
  assert.match(appSource, /lastConversationAt/);
});
