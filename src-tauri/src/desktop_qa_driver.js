// Loaded only by the debug desktop-qa Builder. Drives the real UI and IPC.
(() => {
  try { localStorage.setItem("codex-manager.locale.v1", "zh-CN"); } catch {}
  const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
  const wait = async (fn, message) => {
    const deadline = Date.now() + 15000;
    while (Date.now() < deadline) {
      const result = await fn();
      if (result) return result;
      await sleep(60);
    }
    throw new Error(message);
  };
  const results = { mode: "real-tauri-desktop-artificial-data", checks: [], state: "running" };
  const invoke = (command, args = {}) => window.__TAURI_INTERNALS__.invoke(command, args);
  const emit = (event, payload) => invoke("plugin:event|emit", { event, payload });
  const record = async (name, evidence) => {
    results.checks.push({ name, passed: true, evidence });
    await emit("desktop-qa-report", results);
  };
  const assert = (condition, message) => { if (!condition) throw new Error(message); };
  const button = (text, parent = document) => [...parent.querySelectorAll("button")].find((node) => node.textContent.trim().includes(text));
  const edit = (node, value) => {
    const proto = node.tagName === "TEXTAREA" ? HTMLTextAreaElement.prototype : node.tagName === "SELECT" ? HTMLSelectElement.prototype : HTMLInputElement.prototype;
    Object.getOwnPropertyDescriptor(proto, "value").set.call(node, value);
    node.dispatchEvent(new Event(node.tagName === "SELECT" ? "change" : "input", { bubbles: true }));
  };
  const run = async () => {
    await wait(() => window.__TAURI_INTERNALS__ && document.querySelector(".side-nav"), "real desktop shell did not load");
    const emptyProject = window.__DESKTOP_QA.emptyProject;
    const emptyPath = `${emptyProject}/AGENTS.md`;
    const path = `${window.__DESKTOP_QA.project}/AGENTS.md`;
    const rejected = [];
    for (const command of ["get_codex_account", "start_codex_login", "list_auth_profiles", "import_auth_profile", "apply_codex_config_profile", "restore_codex_config", "list_proxy_auth_profiles", "install_latest_cliproxy_core", "start_codex_gateway", "stop_codex_gateway", "check_for_update", "install_pending_update", "update_settings", "probe_codex"]) {
      let error = null;
      try { await invoke(command); } catch (value) { error = String(value); }
      assert(error && /not found|not allowed|not permitted/i.test(error), `unsafe command did not fail closed: ${command}`);
      rejected.push(command);
    }
    await record("unsafe_commands_have_no_handlers", rejected);
    const facets = await invoke("get_activity_facets", { view: "turns" });
    assert(facets.models.includes("qa-rare-model"), "rare model absent from native facets");
    const page = await invoke("list_activity", { query: { limit: 50, view: "turns" } });
    assert(page.items?.length === 50, "native first page must have 50 items");
    assert(!page.items.some((row) => row.model.value === "qa-rare-model"), "fixture rare model must be beyond first page");
    const nav = document.querySelector(".side-nav");
    button("活动记录", nav).click();
    const select = await wait(() => [...document.querySelectorAll(".filters select")].find((node) => [...node.options].some((option) => option.value === "qa-rare-model")), "rare model option not available in UI");
    edit(select, "qa-rare-model");
    await wait(() => document.querySelector("tbody")?.textContent.includes("qa-rare-model"), "native activity filter did not render rare model");
    await record("cross_page_model_filter_uses_native_sqlite", { model: "qa-rare-model", fixtureRows: 60 });
    button("项目与 AGENTS", nav).click();
    const projectRow = (canonicalPath) => [...document.querySelectorAll(".project-row")].find((row) => row.querySelector(".project-row-main > small")?.textContent === canonicalPath);
    const selectProject = async (canonicalPath) => {
      (await wait(() => projectRow(canonicalPath), `project row unavailable: ${canonicalPath}`)).click();
      await wait(() => projectRow(canonicalPath)?.classList.contains("is-selected"), "project selection did not settle");
    };
    const currentEditor = async (canonicalPath) => wait(() => {
      const node = document.querySelector("#agents-content");
      return document.querySelector(".editor-meta code")?.textContent === `${canonicalPath}/AGENTS.md` && node && !node.readOnly && node;
    }, "AGENTS editor is not ready for the selected file");
    const confirmDialog = async (action) => {
      const dialog = await wait(() => document.querySelector('dialog[open][aria-labelledby="confirmation-title"]'), "confirmation dialog was not shown");
      assert(dialog.getBoundingClientRect().width > 0, "confirmation dialog is not visible");
      const control = action === "cancel" ? button("取消", dialog) : dialog.querySelector('[data-confirmation-action="accept"]');
      assert(control && !control.disabled, "confirmation action is unavailable");
      control.click();
      await wait(() => !document.querySelector('dialog[open][aria-labelledby="confirmation-title"]'), "confirmation dialog did not dismiss");
    };
    await selectProject(window.__DESKTOP_QA.project);
    const editor = await currentEditor(window.__DESKTOP_QA.project);
    const initial = await invoke("open_agents_file", { path });
    const savedText = "# Desktop QA saved through UI\n\n真实 Tauri IPC 安全保存。\n";
    edit(editor, savedText);
    const save = await wait(() => { const found = button("安全保存"); return found && !found.disabled && found; }, "save button not enabled");
    save.click();
    await wait(async () => (await invoke("open_agents_file", { path })).content === savedText, "UI save did not persist via native IPC");
    const saved = await invoke("open_agents_file", { path });
    assert(saved.sha256 !== initial.sha256, "saved hash unchanged");
    await wait(() => document.body.textContent.includes("已与磁盘快照一致"), "save completion did not update editor");
    await record("agents_ui_save_and_readback", { path, shaChanged: true });
    const revisionButton = () => {
      const row = [...document.querySelectorAll(".revision-row")].find((node) => node.textContent.includes(saved.sha256));
      const control = row && button("恢复此版本", row);
      return control && !control.disabled && control;
    };
    (await wait(revisionButton, "saved revision restore action unavailable")).click();
    await confirmDialog("cancel");
    await currentEditor(window.__DESKTOP_QA.project);
    assert((await invoke("open_agents_file", { path })).content === savedText, "cancelled restore changed the saved file");
    (await wait(revisionButton, "restore action remained busy after cancellation")).click();
    await confirmDialog("accept");
    // A revision restores the captured pre-save bytes, as the production
    // restore_agents_inner contract specifies.
    await wait(async () => (await invoke("open_agents_file", { path })).content === initial.content, "confirmed restore did not recover captured pre-save bytes");
    assert((await currentEditor(window.__DESKTOP_QA.project)).value === initial.content, "restored disk content was not reflected in editor");
    await record("agents_restore_real_dialog_cancel_and_confirm", { cancelPreservedDisk: true, confirmedRestoredBeforeSha: initial.sha256 });
    await selectProject(emptyProject);
    const createButton = () => { const control = button("创建项目级 AGENTS.md"); return control && !control.disabled && control; };
    const emptyChain = () => invoke("get_agents_chain", { projectPath: emptyProject, selectedCwd: emptyProject });
    assert(!(await emptyChain()).files.some((file) => file.path === emptyPath), "empty project unexpectedly has AGENTS before create");
    (await wait(createButton, "create AGENTS action unavailable")).click();
    await confirmDialog("cancel");
    await wait(createButton, "create action remained busy after cancellation");
    assert(!(await emptyChain()).files.some((file) => file.path === emptyPath), "cancelled create produced an AGENTS file");
    (await wait(createButton, "create action unavailable after cancellation")).click();
    await confirmDialog("accept");
    await currentEditor(emptyProject);
    const created = await invoke("open_agents_file", { path: emptyPath });
    assert(created.content.includes("# 项目 AGENTS 说明"), "confirmed creation has unexpected content");
    assert((await emptyChain()).files.some((file) => file.path === emptyPath), "created AGENTS absent from native file chain");
    await record("agents_create_real_dialog_cancel_and_confirm", { cancelKeptFileAbsent: true, confirmedPath: emptyPath, chainIncludesCreatedFile: true });
    await selectProject(window.__DESKTOP_QA.project);
    await currentEditor(window.__DESKTOP_QA.project);
    await emit("desktop-qa-control", "external-edit");
    await wait(async () => (await invoke("open_agents_file", { path })).content.includes("external edit"), "artificial external editor did not finish");
    const draft = "# Unsaved QA draft after external change\n";
    edit(document.querySelector("#agents-content"), draft);
    (await wait(() => { const found = button("安全保存"); return found && !found.disabled && found; }, "conflict save not enabled")).click();
    await wait(() => /冲突|已被外部修改/.test(document.querySelector(".notice-error")?.textContent ?? ""), "CAS conflict not shown");
    assert(document.querySelector("#agents-content").value === draft, "conflict discarded draft");
    assert((await invoke("open_agents_file", { path })).content.includes("external edit"), "conflict overwrote external content");
    await record("agents_external_conflict_keeps_disk_and_draft", { draftPreserved: true, externalContentPreserved: true });
    await wait(() => { const found = button("安全保存"); return found && !found.disabled; }, "save operation did not settle after conflict");
    button("活动记录", document.querySelector(".side-nav")).click();
    const navigationDialog = await wait(() => document.querySelector('dialog[open][aria-labelledby="confirmation-title"]'), "dirty navigation confirmation dialog was not shown");
    assert(navigationDialog.getBoundingClientRect().width > 0, "navigation dialog is not visibly rendered");
    const cancelNavigation = button("取消", navigationDialog);
    assert(cancelNavigation, "navigation dialog has no cancel action");
    cancelNavigation.click();
    await wait(() => !document.querySelector('dialog[open][aria-labelledby="confirmation-title"]'), "navigation dialog did not close after cancel");
    assert(document.querySelector(".side-nav button.is-active")?.textContent.includes("项目与 AGENTS"), "cancelled navigation changed the active view");
    assert(document.querySelector("#agents-content")?.value === draft, "cancelled navigation discarded draft");
    await record("dirty_navigation_real_dialog_cancel_keeps_view_and_draft", { dialogVisible: true, cancelledThroughActualButton: true });
    await emit("desktop-qa-control", "close");
    const closeDialog = await wait(() => document.querySelector('dialog[open][aria-labelledby="confirmation-title"]'), "native close did not show the real unsaved-changes dialog");
    assert(closeDialog.getBoundingClientRect().width > 0, "close confirmation dialog is not visibly rendered");
    const cancelClose = button("取消", closeDialog);
    assert(cancelClose, "close dialog has no cancel action");
    cancelClose.click();
    await wait(() => !document.querySelector('dialog[open][aria-labelledby="confirmation-title"]'), "native close confirmation did not dismiss after cancel");
    await sleep(300);
    assert(document.querySelector("#agents-content")?.value === draft, "native close cancellation lost window or draft");
    await record("native_close_requested_real_dialog_cancel_keeps_draft", { dialogVisible: true, cancelledThroughActualButton: true });
    results.closeLifecycle = { cancelled: "verified through real dialog", allowed: "pending visible inspection and manual confirmation" };
    results.state = "awaiting-visible-inspection-and-close-allow";
    results.next = "Inspect the real window, then close and accept discard. window-destroyed.json records the native Destroyed event.";
    await emit("desktop-qa-report", results);
    document.title = "Codex Manager · QA 自动检查通过 · 等待允许关闭验证";
  };
  run().catch(async (error) => {
    results.state = "failed";
    results.error = String(error);
    try { await emit("desktop-qa-report", results); } catch {}
    console.error("DESKTOP_QA_FAILED", error);
  });
})();
