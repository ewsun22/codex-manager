import { useCallback, useEffect, useMemo, useState, type FormEvent } from "react";
import { invokeBackend } from "./client.ts";
import {
  COMMANDS,
  type CodexGatewaySetup,
  type CodexGatewayStatus,
  type CodexProviderProfile,
  type CodexReasoningEffort,
  type SaveCodexProviderInput,
} from "../shared/contracts.ts";

type GatewayNotice = (tone: "success" | "error" | "info", message: string) => void;
type GatewayModal = "provider" | "options" | "setup" | null;

const efforts: CodexReasoningEffort[] = ["minimal", "low", "medium", "high", "xhigh"];

const emptyDraft: SaveCodexProviderInput = {
  name: "",
  baseUrl: "",
  model: "",
  reasoningEffort: "high",
  apiKey: "",
};

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : "操作未完成，请刷新后重试。";
}

function stateLabel(status: CodexGatewayStatus | null): string {
  if (!status) return "正在读取";
  return { stopped: "已停止", running: "运行中", error: "异常" }[status.state];
}

function providerToDraft(provider: CodexProviderProfile): SaveCodexProviderInput {
  return {
    id: provider.id,
    name: provider.name,
    baseUrl: provider.baseUrl,
    model: provider.model,
    reasoningEffort: provider.reasoningEffort,
    apiKey: "",
  };
}

function nonSensitivePreview(draft: SaveCodexProviderInput): string {
  const providerName = draft.name.trim() || "custom-responses";
  const model = draft.model.trim() || "your-model";
  const baseUrl = draft.baseUrl.trim() || "https://api.example.com/v1";
  return [
    "# 预览仅供核对，不会写入 Codex 配置",
    `model = ${JSON.stringify(model)}`,
    `model_provider = ${JSON.stringify(providerName)}`,
    "",
    `[model_providers.${JSON.stringify(providerName)}]`,
    `base_url = ${JSON.stringify(baseUrl)}`,
    'wire_api = "responses"',
    "# API Key 始终省略；仅由原生端写入系统 Keychain。",
  ].join("\n");
}

/** A deliberately small dashboard control. It only exposes the loopback endpoint,
 * never the bearer or a provider API key. */
export function CodexGatewayQuickControl({ onOpenGateway }: { onOpenGateway: () => void }) {
  const [providers, setProviders] = useState<CodexProviderProfile[]>([]);
  const [status, setStatus] = useState<CodexGatewayStatus | null>(null);
  const [selectedId, setSelectedId] = useState<string>("");
  const [busy, setBusy] = useState<"start" | "stop" | null>(null);
  const [message, setMessage] = useState("正在读取本机代理状态…");

  const refresh = useCallback(async () => {
    try {
      const [nextProviders, nextStatus] = await Promise.all([
        invokeBackend<CodexProviderProfile[]>(COMMANDS.listCodexProviders),
        invokeBackend<CodexGatewayStatus>(COMMANDS.getCodexGatewayStatus),
      ]);
      setProviders(nextProviders);
      setStatus(nextStatus);
      setSelectedId((current) => current && nextProviders.some((provider) => provider.id === current && provider.hasApiKey)
        ? current
        : nextStatus.providerId ?? nextProviders.find((provider) => provider.hasApiKey)?.id ?? "");
      setMessage("");
    } catch (error) {
      setMessage(`代理状态不可用：${errorMessage(error)}`);
    }
  }, []);

  useEffect(() => { void refresh(); }, [refresh]);

  const availableProviders = providers.filter((provider) => provider.hasApiKey);
  const selected = availableProviders.find((provider) => provider.id === selectedId) ?? null;
  const running = status?.state === "running";
  const runningProvider = providers.find((provider) => provider.id === status?.providerId) ?? null;
  const endpoint = status ? status.endpoint ?? `http://127.0.0.1:${status.port}/v1` : null;
  const runGateway = async () => {
    if (!selected || !selected.hasApiKey || busy) return;
    setBusy("start");
    setMessage("");
    try {
      const next = await invokeBackend<CodexGatewayStatus>(COMMANDS.startCodexGateway, { providerId: selected.id });
      setStatus(next);
      setMessage(`代理已开启，上游为“${selected.name}”。`);
    } catch (error) {
      setMessage(`代理未开启：${errorMessage(error)}`);
    } finally {
      setBusy(null);
    }
  };
  const stopGateway = async () => {
    if (busy) return;
    setBusy("stop");
    setMessage("");
    try {
      const next = await invokeBackend<CodexGatewayStatus>(COMMANDS.stopCodexGateway);
      setStatus(next);
      setMessage("代理已关闭。已手动应用配置时，请恢复直连配置。");
    } catch (error) {
      setMessage(`代理未关闭：${errorMessage(error)}`);
    } finally {
      setBusy(null);
    }
  };
  const copyEndpoint = async () => {
    if (!endpoint) {
      setMessage("代理状态尚未就绪，暂时不能复制 endpoint。");
      return;
    }
    try {
      await navigator.clipboard.writeText(endpoint);
      setMessage("OpenAI Responses endpoint 已复制。");
    } catch {
      setMessage("系统拒绝写入剪贴板，请手动复制 endpoint。");
    }
  };

  return (
    <section className={`panel gateway-quick-panel${running ? " is-running" : ""}`} aria-labelledby="gateway-quick-heading" aria-busy={busy !== null}>
      <div className="panel-title-row gateway-quick-heading">
        <div>
          <p className="gateway-section-kicker">LOOPBACK ONLY</p>
          <h2 id="gateway-quick-heading">代理 API 接入</h2>
          <p>仅 OpenAI Responses；不提供 Claude 或 Gemini transformer。</p>
        </div>
        <span className={`state-pill state-${running ? "healthy" : status?.state === "error" ? "degraded" : "disabled"}`}>{stateLabel(status)}</span>
      </div>
      <div className="gateway-quick-endpoint">
        <span>OpenAI Responses</span>
        <code title={endpoint ?? undefined}>{endpoint ?? "unavailable"}</code>
        <button type="button" className="button button-secondary" onClick={() => void copyEndpoint()} disabled={!endpoint || busy !== null}>复制 endpoint</button>
      </div>
      <dl className="gateway-quick-meta">
        <div><dt>上游 / 模型</dt><dd>{status?.providerName ?? selected?.name ?? "未选择"}{(running ? runningProvider : selected)?.model ? ` · ${(running ? runningProvider : selected)?.model}` : ""}</dd></div>
        <div><dt>请求 / 失败</dt><dd>{status ? `${status.requests} / ${status.failed}` : "—"}</dd></div>
        <div><dt>处理中</dt><dd>{status?.inFlight ?? "—"}</dd></div>
      </dl>
      {!running ? (
        <div className="gateway-quick-actions">
          {availableProviders.length ? <label><span>可用上游</span><select value={selectedId} onChange={(event) => setSelectedId(event.target.value)} disabled={busy !== null}>{availableProviders.map((provider) => <option key={provider.id} value={provider.id}>{provider.name}</option>)}</select></label> : null}
          {availableProviders.length ? <button type="button" className="button button-primary" onClick={() => void runGateway()} disabled={!selected || busy !== null}>{busy === "start" ? "开启中…" : "开启代理"}</button> : <button type="button" className="button button-primary" onClick={onOpenGateway}>配置供应商</button>}
          <button type="button" className="button button-secondary" onClick={onOpenGateway} disabled={busy !== null}>{availableProviders.length ? "管理配置" : "查看说明"}</button>
        </div>
      ) : <div className="gateway-quick-actions"><button type="button" className="button button-danger" onClick={() => void stopGateway()} disabled={busy !== null}>{busy === "stop" ? "关闭中…" : "关闭代理"}</button><button type="button" className="button button-secondary" onClick={onOpenGateway} disabled={busy !== null}>管理配置</button></div>}
      {status?.lastError ? <p className="gateway-quick-error" role="alert">最近错误：{status.lastError}</p> : null}
      <p className="gateway-quick-message" aria-live="polite">{message}</p>
    </section>
  );
}

export function CodexGatewayView({
  onNotice,
  onOpenOfficialSubscription,
  codexHome,
}: {
  onNotice: GatewayNotice;
  onOpenOfficialSubscription?: () => void;
  codexHome?: string;
}) {
  const [providers, setProviders] = useState<CodexProviderProfile[]>([]);
  const [status, setStatus] = useState<CodexGatewayStatus | null>(null);
  const [draft, setDraft] = useState<SaveCodexProviderInput>(emptyDraft);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [portDraft, setPortDraft] = useState("47653");
  const [setup, setSetup] = useState<CodexGatewaySetup | null>(null);
  const [modal, setModal] = useState<GatewayModal>(null);
  const [openMenuId, setOpenMenuId] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState<string | null>(null);

  const selected = useMemo(
    () => providers.find((provider) => provider.id === selectedId) ?? null,
    [providers, selectedId],
  );
  const editingRunningProvider = status?.state === "running" && draft.id === status.providerId;
  const normalizedCodexHome = codexHome?.trim().replace(/[\\/]+$/, "") ?? "";
  const configPath = normalizedCodexHome
    ? `${normalizedCodexHome}${normalizedCodexHome.includes("\\") && !normalizedCodexHome.includes("/") ? "\\" : "/"}config.toml`
    : "unavailable";

  const refresh = useCallback(async () => {
    setLoading(true);
    try {
      const [nextProviders, nextStatus] = await Promise.all([
        invokeBackend<CodexProviderProfile[]>(COMMANDS.listCodexProviders),
        invokeBackend<CodexGatewayStatus>(COMMANDS.getCodexGatewayStatus),
      ]);
      setProviders(nextProviders);
      setStatus(nextStatus);
      setPortDraft(String(nextStatus.port));
      setSelectedId((current) => current && nextProviders.some((item) => item.id === current)
        ? current
        : nextStatus.providerId ?? nextProviders[0]?.id ?? null);
    } catch (error) {
      onNotice("error", `Codex 网关状态不可用：${errorMessage(error)}`);
    } finally {
      setLoading(false);
    }
  }, [onNotice]);

  useEffect(() => {
    void refresh();
    return () => {
      setSetup(null);
      setModal(null);
    };
  }, [refresh]);

  useEffect(() => {
    if (selected && !draft.id) setDraft(providerToDraft(selected));
  }, [draft.id, selected]);

  const chooseProvider = (provider: CodexProviderProfile) => {
    setSelectedId(provider.id);
    setDraft(providerToDraft(provider));
    setSetup(null);
    setOpenMenuId(null);
  };

  const openCreateProvider = () => {
    setSelectedId(null);
    setDraft(emptyDraft);
    setSetup(null);
    setOpenMenuId(null);
    setModal("provider");
  };

  const openEditProvider = (provider: CodexProviderProfile) => {
    chooseProvider(provider);
    setModal("provider");
  };

  const saveProvider = async (event: FormEvent) => {
    event.preventDefault();
    setBusy("save");
    try {
      const saved = await invokeBackend<CodexProviderProfile>(COMMANDS.saveCodexProvider, {
        input: {
          ...draft,
          name: draft.name.trim(),
          baseUrl: draft.baseUrl.trim(),
          model: draft.model.trim(),
          apiKey: draft.apiKey?.trim() || undefined,
        },
      });
      const next = await invokeBackend<CodexProviderProfile[]>(COMMANDS.listCodexProviders);
      setProviders(next);
      setSelectedId(saved.id);
      setDraft(providerToDraft(saved));
      setModal(null);
      onNotice("success", `供应商“${saved.name}”已保存；API Key 仅存放在系统 Keychain。`);
    } catch (error) {
      onNotice("error", `供应商未保存：${errorMessage(error)}`);
    } finally {
      setBusy(null);
    }
  };

  const deleteProvider = async (provider: CodexProviderProfile) => {
    if (!window.confirm(`删除供应商“${provider.name}”及其 Keychain API Key？此操作不可恢复。`)) return;
    setBusy(`delete:${provider.id}`);
    try {
      const next = await invokeBackend<CodexProviderProfile[]>(COMMANDS.deleteCodexProvider, {
        providerId: provider.id,
      });
      setProviders(next);
      const nextSelected = next[0] ?? null;
      setSelectedId(nextSelected?.id ?? null);
      setDraft(nextSelected ? providerToDraft(nextSelected) : emptyDraft);
      setOpenMenuId(null);
      onNotice("success", `供应商“${provider.name}”已删除。`);
    } catch (error) {
      onNotice("error", `供应商未删除：${errorMessage(error)}`);
    } finally {
      setBusy(null);
    }
  };

  const updatePort = async () => {
    setBusy("port");
    try {
      const next = await invokeBackend<CodexGatewayStatus>(COMMANDS.updateCodexGatewayPort, {
        port: Number(portDraft),
      });
      setStatus(next);
      setPortDraft(String(next.port));
      setSetup(null);
      onNotice("success", `本地网关端口已设置为 ${next.port}。`);
    } catch (error) {
      onNotice("error", `端口未更新：${errorMessage(error)}`);
    } finally {
      setBusy(null);
    }
  };

  const startGateway = async (provider: CodexProviderProfile) => {
    setBusy("start");
    try {
      const next = await invokeBackend<CodexGatewayStatus>(COMMANDS.startCodexGateway, {
        providerId: provider.id,
      });
      setStatus(next);
      setSelectedId(provider.id);
      setDraft(providerToDraft(provider));
      setSetup(null);
      setOpenMenuId(null);
      onNotice("success", `Codex Responses 网关已启动，上游为“${provider.name}”。`);
    } catch (error) {
      onNotice("error", `网关未启动：${errorMessage(error)}`);
    } finally {
      setBusy(null);
    }
  };

  const stopGateway = async () => {
    setBusy("stop");
    try {
      const next = await invokeBackend<CodexGatewayStatus>(COMMANDS.stopCodexGateway);
      setStatus(next);
      setSetup(null);
      onNotice("success", "Codex Responses 网关已停止。若你已手动应用配置，请先恢复直连配置。" );
    } catch (error) {
      onNotice("error", `网关未停止：${errorMessage(error)}`);
    } finally {
      setBusy(null);
    }
  };

  const revealSetup = async () => {
    setBusy("setup");
    try {
      const revealed = await invokeBackend<CodexGatewaySetup>(COMMANDS.revealCodexGatewaySetup);
      setSetup(revealed);
      setModal("setup");
      onNotice("info", "已临时显示 Codex 配置片段；其中包含仅用于本机 loopback 的访问凭据。" );
    } catch (error) {
      onNotice("error", `无法生成配置片段：${errorMessage(error)}`);
    } finally {
      setBusy(null);
    }
  };

  const copySetup = async () => {
    if (!setup) return;
    try {
      await navigator.clipboard.writeText(setup.configSnippet);
      onNotice("success", "配置片段已复制；粘贴完成后请清理剪贴板。" );
    } catch {
      onNotice("error", "系统拒绝写入剪贴板，请手动选择配置片段。" );
    }
  };

  const testProvider = (provider: CodexProviderProfile) => {
    onNotice("info", `“${provider.name}”的模型测试尚未开放；为避免在未确认前发送请求，当前未连接上游。`);
  };

  const closeModal = () => {
    if (busy === "save" || busy === "setup") return;
    setModal(null);
  };

  useEffect(() => {
    if (!modal && !openMenuId) return;
    const closeOnEscape = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      if (modal) closeModal();
      else setOpenMenuId(null);
    };
    window.addEventListener("keydown", closeOnEscape);
    return () => window.removeEventListener("keydown", closeOnEscape);
  }, [busy, modal, openMenuId]);

  return (
    <div className="view-content gateway-view">
      <div className="gateway-page-heading">
        <div className="gateway-heading-copy">
          <p className="eyebrow">CODEX MANAGER</p>
          <h1>Codex 配置管理</h1>
          <p>集中管理自定义 Responses 上游；应用时只启动本机 loopback 网关，不会改写你的 Codex 配置。</p>
          <p className="gateway-config-location"><span>配置文件位置</span><code>{configPath}</code></p>
        </div>
        <div className="gateway-heading-actions">
          <button type="button" className="button button-secondary" onClick={() => void refresh()} disabled={loading || busy !== null}>{loading ? "刷新中…" : "刷新"}</button>
          <button type="button" className="button button-secondary" onClick={() => setModal("options")} disabled={busy !== null}>更多选项</button>
        </div>
      </div>

      <section className="panel gateway-provider-panel" aria-labelledby="gateway-providers-heading">
        <div className="gateway-provider-heading">
          <div>
            <p className="gateway-section-kicker">自定义供应商</p>
            <h2 id="gateway-providers-heading">供应商配置</h2>
            <p>API Key 只写入系统 Keychain；列表和本地数据库不会保存或回显密钥。</p>
          </div>
          <button type="button" className="button button-primary" onClick={openCreateProvider} disabled={busy !== null}>新增供应商</button>
        </div>
        {loading && !providers.length ? (
          <div className="loading-state" role="status" aria-live="polite"><span className="loading-mark" aria-hidden="true" />正在读取本地供应商元数据…</div>
        ) : providers.length ? (
          <div className="gateway-provider-cards">
            <div className="gateway-provider-columns" aria-hidden="true">
              <span>供应商</span>
              <span>模型 / 推理</span>
              <span>凭据</span>
              <span>状态</span>
              <span>操作</span>
            </div>
            {providers.map((provider) => {
              const running = status?.state === "running" && status.providerId === provider.id;
              const hasOpenMenu = openMenuId === provider.id;
              return <article key={provider.id} className={`gateway-provider-card${running ? " is-applied" : ""}`}>
                <div className="gateway-provider-identity">
                  <h3>{provider.name}</h3>
                  <code title={provider.baseUrl}>{provider.baseUrl}</code>
                </div>
                <div className="gateway-provider-cell"><span>模型 / 推理</span><strong>{provider.model}</strong><small>{provider.reasoningEffort}</small></div>
                <div className="gateway-provider-cell"><span>凭据</span><strong>{provider.hasApiKey ? "系统 Keychain" : "未保存"}</strong><small>{provider.hasApiKey ? "API Key 已保存" : "需要 API Key"}</small></div>
                <div className="gateway-provider-state"><span>状态</span><span className={`state-pill state-${running ? "healthy" : provider.hasApiKey ? "disabled" : "unavailable"}`}>{running ? "已应用" : provider.hasApiKey ? "可应用" : "缺少 Key"}</span></div>
                <div className="gateway-provider-card-actions">
                  <button type="button" className="button button-secondary" onClick={() => testProvider(provider)} disabled={busy !== null} title="模型测试尚未开放">测试说明</button>
                  <button type="button" className="button button-primary" onClick={() => void startGateway(provider)} disabled={running || !provider.hasApiKey || busy !== null}>{running ? "已应用" : busy === "start" ? "应用中…" : "应用"}</button>
                  <button type="button" className="button button-secondary gateway-more-button" onClick={() => setOpenMenuId(hasOpenMenu ? null : provider.id)} aria-expanded={hasOpenMenu} aria-controls={`provider-menu-${provider.id}`} disabled={busy !== null}>更多</button>
                  {hasOpenMenu ? <div id={`provider-menu-${provider.id}`} className="gateway-card-menu" role="menu">
                    <button type="button" role="menuitem" onClick={() => openEditProvider(provider)}>编辑供应商</button>
                    <button type="button" role="menuitem" className="danger-text" onClick={() => void deleteProvider(provider)} disabled={running || busy !== null}>{busy === `delete:${provider.id}` ? "删除中…" : "删除供应商"}</button>
                  </div> : null}
                </div>
              </article>;
            })}
          </div>
        ) : <div className="empty-state gateway-provider-empty"><strong>还没有自定义供应商</strong><p>添加一个 OpenAI Responses-compatible 上游后，才能将它应用为本机透明网关。</p><button type="button" className="button button-primary" onClick={openCreateProvider}>添加供应商</button></div>}
        <article className="gateway-provider-card gateway-official-card">
          <div className="gateway-provider-identity">
            <h3>默认配置</h3>
            <code>官方订阅 · 使用 Codex 当前登录态</code>
          </div>
          <div className="gateway-provider-cell"><span>模型 / 推理</span><strong>Codex OAuth</strong><small>由官方配置决定</small></div>
          <div className="gateway-provider-cell"><span>凭据</span><strong>官方受信任链路</strong><small>网关永不接入</small></div>
          <div className="gateway-provider-state"><span>状态</span><span className="state-pill state-disabled">独立管理</span></div>
          <div className="gateway-provider-card-actions">
            <button type="button" className="button button-secondary" onClick={onOpenOfficialSubscription}>管理官方订阅</button>
          </div>
        </article>
      </section>

      <details className={`panel gateway-status-panel gateway-status-${status?.state ?? "loading"}`} open={status?.state === "running"} aria-busy={loading}>
        <summary className="gateway-status-summary">
          <span><strong>本地 Responses 网关</strong><small>{status?.endpoint ?? `http://127.0.0.1:${status?.port ?? portDraft}/v1`} · {status?.providerName ?? "未选择上游"}</small></span>
          <span className={`state-pill state-${status?.state === "running" ? "healthy" : status?.state === "error" ? "degraded" : "disabled"}`}>{stateLabel(status)}</span>
        </summary>
        <div className="gateway-status-grid">
          <div><span>监听地址</span><strong>{status?.endpoint ?? `http://127.0.0.1:${status?.port ?? portDraft}/v1`}</strong></div>
          <div><span>当前上游</span><strong>{status?.providerName ?? "未选择"}</strong></div>
          <div><span>请求 / 失败</span><strong>{status ? `${status.requests} / ${status.failed}` : "—"}</strong></div>
          <div><span>处理中</span><strong>{status?.inFlight ?? "—"}</strong></div>
        </div>
        {status?.lastError ? <p className="gateway-error" role="alert">最近错误：{status.lastError}</p> : null}
        <div className="gateway-toolbar">
          <label><span>固定端口</span><input type="number" min="1024" max="65535" value={portDraft} onChange={(event) => setPortDraft(event.target.value)} disabled={status?.state === "running" || busy !== null} /></label>
          <button type="button" className="button button-secondary" onClick={() => void updatePort()} disabled={status?.state === "running" || busy !== null}>{busy === "port" ? "保存中…" : "保存端口"}</button>
          {status?.state === "running" ? (
            <>
              <button type="button" className="button button-secondary" onClick={() => void revealSetup()} disabled={busy !== null}>{busy === "setup" ? "生成中…" : "显示配置片段"}</button>
              <button type="button" className="button button-danger" onClick={() => void stopGateway()} disabled={busy !== null}>{busy === "stop" ? "停止中…" : "停止网关"}</button>
            </>
          ) : <span className="gateway-toolbar-hint">从供应商卡片点击“应用”后，网关才会启动。</span>}
        </div>
      </details>

      <section className="gateway-capability-stack" aria-label="更多配置能力">
        <details className="gateway-capability">
          <summary><span>官方订阅</span><small>入口说明</small></summary>
          <div><p>官方订阅、账户切换与额度只在“官方订阅”工作台管理。本页不会读取、复制或代理官方 OAuth token，也不会展示未经观测的额度。</p></div>
        </details>
      </section>

      {modal === "provider" ? (
        <div className="gateway-modal-backdrop" role="presentation" onMouseDown={closeModal}>
          <form className="gateway-modal provider-form" aria-labelledby="provider-modal-heading" aria-modal="true" role="dialog" onMouseDown={(event) => event.stopPropagation()} onSubmit={(event) => void saveProvider(event)}>
            <div className="gateway-modal-header">
              <div><p className="eyebrow">自定义供应商</p><h2 id="provider-modal-heading">{draft.id ? "编辑供应商" : "新增供应商"}</h2><p>{draft.id ? "API Key 留空会保留系统 Keychain 中的现有值。" : "只支持 OpenAI Responses-compatible 上游。"}</p></div>
              <button type="button" className="gateway-modal-close" onClick={closeModal} aria-label="关闭供应商编辑">关闭</button>
            </div>
            {editingRunningProvider ? <p className="gateway-error" role="status">该供应商正在被网关使用。请先停止网关，再修改地址、模型或 API Key。</p> : null}
            <div className="gateway-modal-chips"><span>渠道：自定义</span><span>协议：OpenAI Responses</span></div>
            <div className="provider-form-fields">
              <label><span>名称</span><input value={draft.name} maxLength={80} required onChange={(event) => setDraft({ ...draft, name: event.target.value })} placeholder="例如：公司 Responses 中转" /></label>
              <label><span>Base URL</span><input value={draft.baseUrl} required onChange={(event) => setDraft({ ...draft, baseUrl: event.target.value })} placeholder="https://api.example.com/v1" /></label>
              <label><span>模型</span><input value={draft.model} maxLength={128} required onChange={(event) => setDraft({ ...draft, model: event.target.value })} placeholder="gpt-5.6-sol" /></label>
              <label><span>推理等级</span><select value={draft.reasoningEffort} onChange={(event) => setDraft({ ...draft, reasoningEffort: event.target.value as CodexReasoningEffort })}>{efforts.map((effort) => <option key={effort} value={effort}>{effort}</option>)}</select></label>
              <label className="provider-key-field"><span>API Key</span><input type="password" autoComplete="off" value={draft.apiKey ?? ""} onChange={(event) => setDraft({ ...draft, apiKey: event.target.value })} placeholder={draft.id ? "留空以保留现有 Key" : "仅写入系统 Keychain"} /></label>
            </div>
            <section className="gateway-preview" aria-labelledby="gateway-preview-heading"><div><h3 id="gateway-preview-heading">非敏感配置预览</h3><p>仅随表单变化；保存后也不会自动写入 Codex 文件。</p></div><pre>{nonSensitivePreview(draft)}</pre><p>真正的 loopback bearer 不会显示在这里，需在网关运行后由原生端按需确认并单独显示。</p></section>
            <div className="provider-form-actions"><button type="button" className="button button-secondary" onClick={() => setDraft(emptyDraft)} disabled={busy !== null}>清空</button><button type="submit" className="button button-primary" disabled={busy !== null || editingRunningProvider}>{busy === "save" ? "保存中…" : "保存供应商"}</button></div>
            <p className="provider-form-note">远程上游必须使用 HTTPS；仅 `localhost`、`127.0.0.1` 和 `::1` 可使用 HTTP。URL 中不允许用户名、密码、query 或 fragment。</p>
          </form>
        </div>
      ) : null}

      {modal === "options" ? (
        <div className="gateway-modal-backdrop" role="presentation" onMouseDown={closeModal}>
          <section className="gateway-modal gateway-options-modal" aria-labelledby="gateway-options-heading" aria-modal="true" role="dialog" onMouseDown={(event) => event.stopPropagation()}>
            <div className="gateway-modal-header"><div><p className="eyebrow">更多选项</p><h2 id="gateway-options-heading">配置能力预览</h2><p>下列开关尚未开放，均不会改变本机或 Codex 行为。</p></div><button type="button" className="gateway-modal-close" onClick={closeModal} aria-label="关闭更多选项">关闭</button></div>
            <div className="gateway-disabled-options">
              <label><span><strong>自动写入 `config.toml`</strong><small>尚未开放；当前始终由你手动应用配置片段。</small></span><input type="checkbox" disabled /></label>
              <label><span><strong>自动切换官方账户</strong><small>尚未开放；本页不会访问 OAuth 凭据。</small></span><input type="checkbox" disabled /></label>
              <label><span><strong>请求日志正文</strong><small>已禁用；本应用只保留受限元数据。</small></span><input type="checkbox" disabled /></label>
            </div>
          </section>
        </div>
      ) : null}

      {modal === "setup" && setup ? (
        <div className="gateway-modal-backdrop" role="presentation" onMouseDown={closeModal}>
          <section className="gateway-modal gateway-setup-modal" aria-labelledby="gateway-setup-heading" aria-modal="true" role="dialog" onMouseDown={(event) => event.stopPropagation()}>
            <div className="gateway-modal-header"><div><p className="eyebrow">本机 loopback</p><h2 id="gateway-setup-heading">Codex 配置片段</h2><p>这是由原生端按需生成的临时片段；应用不会自动改写 `config.toml` 或 `auth.json`。</p></div><button type="button" className="gateway-modal-close" onClick={closeModal} aria-label="关闭配置片段">关闭</button></div>
            <pre>{setup.configSnippet}</pre>
            <div className="gateway-setup-actions"><p>片段含仅用于本机 loopback 的 bearer。使用后请清理剪贴板，停止网关前恢复原配置。</p><button type="button" className="button button-primary" onClick={() => void copySetup()}>复制片段</button></div>
          </section>
        </div>
      ) : null}
    </div>
  );
}
