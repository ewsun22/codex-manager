import assert from "node:assert/strict";
import { test } from "node:test";
import { AgentsDraft } from "../src/app/agents-draft.ts";
import { ConfirmationAction, ConfirmationState } from "../src/app/confirmation-state.ts";
import type { AgentsFileSnapshot } from "../src/shared/contracts.ts";

const request = { title: "fixture confirmation", message: "fixture action target", confirmLabel: "Confirm" };
const base: AgentsFileSnapshot = { path: "/fixture/AGENTS.md", relativePath: "AGENTS.md", kind: "project", precedence: 40, effective: true, overridden: false, sha256: "base", mtimeMs: 1, sizeBytes: 6, writable: true, content: "# base", newline: "lf" };

test("确认需要真实响应，第二次请求不会拿到首个动作的批准", async () => {
  const state = new ConfirmationState();
  let settled = false;
  const first = state.request(request).then((answer) => { settled = true; return answer; });
  const id = state.getSnapshot()!.id;
  await Promise.resolve();
  assert.equal(settled, false);
  assert.equal(await state.request({ ...request, message: "different target" }), false);
  assert.equal(state.getSnapshot()?.message, request.message);
  state.answer(id, true);
  assert.equal(await first, true);
  assert.equal(state.getSnapshot(), null);
});

test("取消保留完整草稿，确认离开后才放弃；过期按钮事件不能取消新对话框", async () => {
  const state = new ConfirmationState();
  const draft = new AgentsDraft();
  draft.open(base);
  draft.edit("# unsaved", false);
  const cancelled = draft.canLeave(false, () => state.request(request));
  const oldId = state.getSnapshot()!.id;
  state.answer(oldId, false);
  assert.equal(await cancelled, false);
  assert.equal(draft.content, "# unsaved");
  assert.equal(draft.snapshot?.sha256, "base");

  const accepted = draft.canLeave(false, () => state.request(request));
  const id = state.getSnapshot()!.id;
  state.answer(oldId, true);
  assert.equal(state.getSnapshot()?.id, id);
  state.answer(id, true);
  assert.equal(await accepted, true);
  assert.equal(draft.dirty, false);
});

test("确认期间发生的新编辑不会被旧批准丢弃，卸载会取消等待", async () => {
  const state = new ConfirmationState();
  const draft = new AgentsDraft();
  draft.open(base);
  draft.edit("# first edit", false);
  const pending = draft.canLeave(false, () => state.request(request));
  draft.edit("# newer edit", false);
  state.answer(state.getSnapshot()!.id, true);
  assert.equal(await pending, false);
  assert.equal(draft.content, "# newer edit");
  const closing = draft.canLeave(false, () => state.request(request));
  state.cancel();
  assert.equal(await closing, false);
  assert.equal(draft.content, "# newer edit");
});

test("导航确认期间的另一次选择、关闭或保存立即拒绝，不会串动作", async () => {
  const state = new ConfirmationState();
  const action = new ConfirmationAction();
  const draft = new AgentsDraft();
  draft.open(base);
  draft.edit("# unsaved", false);
  const executed: string[] = [];
  const navigation = action.run(async () => {
    if (!await draft.canLeave(false, () => state.request(request))) return false;
    executed.push("first navigation");
    return true;
  });
  for (const name of ["other project", "window close", "save"]) {
    assert.equal(await action.run(async () => { executed.push(name); return true; }), false);
  }
  state.answer(state.getSnapshot()!.id, true);
  assert.equal(await navigation, true);
  assert.deepEqual(executed, ["first navigation"]);
  assert.equal(action.pending, false);
});

test("创建或恢复的取消不会调用写入，批准后写入期间仍阻止关闭", async () => {
  const state = new ConfirmationState();
  const action = new ConfirmationAction();
  let writes = 0;
  let completeWrite!: () => void;
  const perform = () => action.run(async () => {
    if (!await state.request(request)) return false;
    writes += 1;
    await new Promise<void>((resolve) => { completeWrite = resolve; });
    return true;
  });
  const cancelled = perform();
  state.answer(state.getSnapshot()!.id, false);
  assert.equal(await cancelled, false);
  assert.equal(writes, 0);

  const accepted = perform();
  state.answer(state.getSnapshot()!.id, true);
  await Promise.resolve();
  assert.equal(writes, 1);
  assert.equal(await action.run(async () => { throw new Error("close must not execute"); }), false);
  completeWrite();
  assert.equal(await accepted, true);
  assert.equal(action.pending, false);
});

test("页面卸载会作废已经回答但尚未执行的批准，重新挂载不复用旧批准", async () => {
  const state = new ConfirmationState();
  const action = new ConfirmationAction();
  const writes: string[] = [];
  const perform = (target: string) => action.run(async () => { writes.push(target); return true; }, () => state.request({ ...request, message: target }));
  const pending = perform("old page target");
  assert.equal(await perform("overlapping target"), false);
  state.answer(state.getSnapshot()!.id, true);
  action.deactivate();
  state.cancel();
  action.activate();
  assert.equal(await pending, false);
  assert.deepEqual(writes.slice(), []);

  const current = perform("new page target");
  state.answer(state.getSnapshot()!.id, true);
  assert.equal(await current, true);
  assert.deepEqual(writes, ["new page target"]);

  action.deactivate();
  assert.equal(await action.run(async () => { writes.push("unmounted"); return true; }), false);
  assert.deepEqual(writes, ["new page target"]);
});

test("确认与其他保存、刷新、启停共享互斥，取消与卸载均不写入", async () => {
  const state = new ConfirmationState();
  const action = new ConfirmationAction();
  let writes = 0;
  const pending = action.run(async () => { writes += 1; return true; }, () => state.request(request));
  assert.equal(await action.run(async () => { writes += 100; return true; }), false);
  action.deactivate();
  state.cancel();
  assert.equal(await pending, false);
  assert.equal(writes, 0);
  assert.equal(action.pending, false);
});
