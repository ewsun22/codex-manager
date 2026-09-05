import assert from "node:assert/strict";
import { test } from "node:test";
import { AgentsDraft } from "../src/app/agents-draft.ts";
import { facetOptions, queryForView } from "../src/app/activity-filters.ts";
import { invokeDemo } from "../src/app/demo.ts";
import { LatestRequest, commitAndRefresh, safeErrorMessage } from "../src/app/operations.ts";
import { StatusChannel, pollWhileVisible } from "../src/app/status-channel.ts";
import { COMMANDS, type ActivityFacets, type ActivityPage, type AgentsFileSnapshot, type BootstrapPayload } from "../src/shared/contracts.ts";

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (error: unknown) => void;
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}

function snapshot(content = "# base", sha256 = "base", path = "/fixture/AGENTS.md"): AgentsFileSnapshot {
  return { path, content, sha256, mtimeMs: 1, sizeBytes: content.length, writable: true, newline: "lf", kind: "project", relativePath: "AGENTS.md", precedence: 40, effective: true, overridden: false };
}

test("晚到的项目/文件读取及失败都不能覆盖当前选择", async () => {
  const requests = new LatestRequest();
  const first = deferred<string>();
  const second = deferred<string>();
  const committed: string[] = [];
  const readA = requests.run(() => first.promise, (value) => committed.push(value));
  const readB = requests.run(() => second.promise, (value) => committed.push(value));
  second.resolve("project B");
  assert.deepEqual(await readB, { status: "applied" });
  first.reject("late project A error");
  assert.deepEqual(await readA, { status: "ignored" });
  assert.deepEqual(committed, ["project B"]);

  const third = deferred<string>();
  const readC = requests.run(() => third.promise, (value) => committed.push(value));
  requests.invalidate();
  third.resolve("closed view");
  assert.deepEqual(await readC, { status: "ignored" });
  assert.deepEqual(committed, ["project B"]);
});

test("AGENTS 取消离开保留草稿，外部新快照不会覆盖内容或 CAS 基准", () => {
  const draft = new AgentsDraft();
  draft.open(snapshot());
  draft.edit("# unsaved", false);
  assert.equal(draft.canLeave(false, () => false), false);
  draft.open(snapshot("# external", "external"));
  assert.equal(draft.content, "# unsaved");
  assert.equal(draft.snapshot?.sha256, "base");
  assert.equal(draft.dirty, true);
  assert.equal(draft.canLeave(false, () => true), true);
  assert.equal(draft.content, "# base");
  draft.open(snapshot("# next", "next", "/fixture-b/AGENTS.md"));
  assert.equal(draft.content, "# next");
});

test("AGENTS 写入期间拒绝输入与导航，失败保留草稿，成功更新基准", async () => {
  const draft = new AgentsDraft();
  draft.open(snapshot());
  draft.edit("# submitted", false);
  let asked = false;
  draft.edit("# typed while saving", true);
  assert.equal(draft.canLeave(true, () => { asked = true; return true; }), false);
  assert.equal(asked, false);
  assert.equal(draft.content, "# submitted");
  let refreshed = false;
  await assert.rejects(commitAndRefresh(
    async () => { throw new Error("external conflict"); },
    (saved: AgentsFileSnapshot) => draft.accept(saved),
    "saved",
    async () => { refreshed = true; },
  ), /external conflict/);
  assert.equal(draft.dirty, true);
  assert.equal(refreshed, false);
  const notice = await commitAndRefresh(
    async () => snapshot(draft.content, "saved"),
    (saved) => draft.accept(saved),
    "saved",
    async () => { throw "revision read failed"; },
  );
  assert.equal(draft.dirty, false);
  assert.equal(draft.snapshot?.sha256, "saved");
  assert.equal(notice.tone, "info");
  assert.match(notice.message, /saved.*revision read failed/);
});

test("错误保留可操作的原生字符串，但去除 URL 查询和凭据", () => {
  assert.equal(safeErrorMessage("Codex 配置已变更；请先重新预览。"), "Codex 配置已变更；请先重新预览。");
  const message = safeErrorMessage(new Error("fixture https://fixture-user:fixture-password@example.test/v1?code=fixture-code#fixture-fragment Bearer fixture-bearer api_key=fixture-key sk-fixture-value"));
  for (const secret of ["fixture-user", "fixture-password", "fixture-code", "fixture-fragment", "fixture-bearer", "fixture-key", "sk-fixture-value"]) assert.equal(message.includes(secret), false);
  assert.match(message, /https:\/\/example.test\/v1/);
  assert.doesNotMatch(safeErrorMessage({ message: "fixture-opaque-payload" }), /fixture-opaque-payload/);
  assert.doesNotMatch(safeErrorMessage('Cookie: session=fixture-cookie; other=fixture-second\n{"access_token":"fixture-json-token"}'), /fixture-cookie|fixture-second|fixture-json-token/);
});

test("全量筛选独立于分页，零结果仍保留选择，视图变化清理不兼容结果", async () => {
  const facets = await invokeDemo<ActivityFacets>(COMMANDS.getActivityFacets, { view: "turns" });
  const first = await invokeDemo<ActivityPage>(COMMANDS.listActivity, { query: { view: "turns", limit: 1 } });
  const all = await invokeDemo<ActivityPage>(COMMANDS.listActivity, { query: { view: "turns", limit: 100 } });
  const bootstrap = await invokeDemo<BootstrapPayload>(COMMANDS.bootstrap);
  assert.ok(bootstrap.activity.items.every((item) => item.activityKind === "turn"));
  assert.equal(bootstrap.summary?.records, all.total);
  assert.deepEqual(facets.models, [...new Set(all.items.flatMap((item) => item.model.value ? [item.model.value] : []))].sort());
  assert.ok(facets.models.some((model) => !first.items.some((item) => item.model.value === model)));
  assert.deepEqual(facetOptions([], "model-selected"), ["model-selected"]);
  const next = queryForView({ view: "modelCalls", model: "model-selected", result: "accounted", cursor: "cursor" }, "turns");
  assert.equal(next.result, null);
  assert.equal(next.cursor, null);
  assert.equal(next.model, "model-selected");
  assert.equal(queryForView({ result: "failure" }, "modelCalls").result, "failure");
});

test("代理共享状态只有一个读取在途，启停前旧响应不能回写状态", async () => {
  const channel = new StatusChannel<string>();
  const pending = deferred<string>();
  let reads = 0;
  const load = () => { reads += 1; return pending.promise; };
  const first = channel.refresh(load);
  const second = channel.refresh(load);
  assert.equal(first, second);
  await Promise.resolve();
  assert.equal(reads, 1);
  channel.publish("stopped after mutation");
  pending.resolve("old running");
  await first;
  assert.equal(channel.getSnapshot().value, "stopped after mutation");
  await channel.refresh(async () => "error: core exited");
  assert.equal(channel.getSnapshot().value, "error: core exited");
  const checkedAt = channel.getSnapshot().checkedAt;
  await assert.rejects(channel.refresh(async () => { throw new Error("status read unavailable"); }), /status read unavailable/);
  assert.equal(channel.getSnapshot().value, "error: core exited");
  assert.equal(channel.getSnapshot().checkedAt, checkedAt);
  assert.equal(channel.getSnapshot().error, "status read unavailable");
});

test("代理轮询仅在页面可见时读取状态，切后台及卸载均停止额外读取", async () => {
  let visible = true;
  let reads = 0;
  let tick = () => {};
  let visibilityChanged = () => {};
  let timerCancelled = false;
  let listenerRemoved = false;
  const stop = pollWhileVisible({
    visible: () => visible,
    refresh: async () => { reads += 1; },
    schedule: (next) => { tick = next; return () => { timerCancelled = true; }; },
    subscribeVisibility: (next) => { visibilityChanged = next; return () => { listenerRemoved = true; }; },
  });
  tick(); assert.equal(reads, 1);
  visible = false; tick(); visibilityChanged(); assert.equal(reads, 1);
  visible = true; visibilityChanged(); assert.equal(reads, 2);
  stop(); tick(); visibilityChanged(); assert.equal(reads, 2);
  assert.equal(timerCancelled, true);
  assert.equal(listenerRemoved, true);
});
