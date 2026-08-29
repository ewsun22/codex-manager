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

export function CodexGatewayView({ onNotice }: { onNotice: GatewayNotice }) {
  const [providers, setProviders] = useState<CodexProviderProfile[]>([]);
  const [status, setStatus] = useState<CodexGatewayStatus | null>(null);
  const [draft, setDraft] = useState<SaveCodexProviderInput>(emptyDraft);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [portDraft, setPortDraft] = useState("47653");
  const [setup, setSetup] = useState<CodexGatewaySetup | null>(null);
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState<string | null>(null);

  const selected = useMemo(
    () => providers.find((provider) => provider.id === selectedId) ?? null,
    [providers, selectedId],
  );
  const editingRunningProvider = status?.state === "running" && draft.id === status.providerId;

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
    return () => setSetup(null);
  }, [refresh]);

  useEffect(() => {
    if (selected && !draft.id) setDraft(providerToDraft(selected));
  }, [draft.id, selected]);

  const chooseProvider = (provider: CodexProviderProfile) => {
    setSelectedId(provider.id);
    setDraft(providerToDraft(provider));
    setSetup(null);
  };

  const createProvider = () => {
    setSelectedId(null);
    setDraft(emptyDraft);
    setSetup(null);
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

  return (
    <div className="view-content gateway-view">
      <div className="section-heading">
        <div>
          <p className="eyebrow">CODEX MANAGER</p>
          <h1>Codex 供应商与反代</h1>
          <p>管理 Responses-compatible 上游，并按需启动仅监听本机的 Codex 透明网关。</p>
        </div>
        <button type="button" className="button button-secondary" onClick={() => void refresh()} disabled={loading || busy !== null}>
          {loading ? "刷新中…" : "刷新状态"}
        </button>
      </div>

      <section className={`panel gateway-status-panel gateway-status-${status?.state ?? "loading"}`}>
        <div className="panel-title-row">
          <div>
            <h2>本地 Responses 网关</h2>
            <p>只接受 `/v1/responses` 与 `/v1/responses/compact`；不接入官方订阅 token，不做账号轮询或跨协议转换。</p>
          </div>
          <span className={`state-pill state-${status?.state === "running" ? "healthy" : status?.state === "error" ? "degraded" : "disabled"}`}>
            {stateLabel(status)}
          </span>
        </div>
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
              <button type="button" className="button button-secondary" onClick={() => void revealSetup()} disabled={busy !== null}>{busy === "setup" ? "生成中…" : "显示 Codex 配置"}</button>
              <button type="button" className="button button-danger" onClick={() => void stopGateway()} disabled={busy !== null}>{busy === "stop" ? "停止中…" : "停止网关"}</button>
            </>
          ) : (
            <button type="button" className="button button-primary" onClick={() => selected && void startGateway(selected)} disabled={!selected?.hasApiKey || busy !== null}>{busy === "start" ? "启动中…" : "使用所选供应商启动"}</button>
          )}
        </div>
      </section>

      {setup ? (
        <section className="panel gateway-setup-panel" aria-labelledby="gateway-setup-heading">
          <div className="panel-title-row">
            <div><h2 id="gateway-setup-heading">Codex 配置片段</h2><p>此片段含本地 client bearer。应用当前不会自动改写 `config.toml` 或 `auth.json`。</p></div>
            <div className="gateway-inline-actions"><button type="button" className="button button-secondary" onClick={() => void copySetup()}>复制</button><button type="button" className="text-button" onClick={() => setSetup(null)}>隐藏</button></div>
          </div>
          <pre>{setup.configSnippet}</pre>
          <p className="gateway-sensitive-note">只在网关运行期间使用；停止网关前先把 Codex 恢复到原供应商，并在粘贴后清理剪贴板。</p>
        </section>
      ) : null}

      <div className="gateway-layout">
        <section className="panel table-panel gateway-provider-panel" aria-labelledby="gateway-providers-heading">
          <div className="panel-title-row">
            <div><h2 id="gateway-providers-heading">第三方供应商</h2><p>API Key 只保存在系统 Keychain，表格和 SQLite 仅保留元数据。</p></div>
            <button type="button" className="button button-secondary" onClick={createProvider}>新增</button>
          </div>
          {providers.length ? (
            <div className="table-scroll"><table className="gateway-provider-table"><thead><tr><th>供应商</th><th>模型</th><th>凭据</th><th>状态</th><th>操作</th></tr></thead><tbody>
              {providers.map((provider) => {
                const running = status?.state === "running" && status.providerId === provider.id;
                return <tr key={provider.id} className={selectedId === provider.id ? "is-selected" : ""}>
                  <td><button type="button" className="provider-select" onClick={() => chooseProvider(provider)}><strong>{provider.name}</strong><small>{provider.baseUrl}</small></button></td>
                  <td><strong>{provider.model}</strong><small>{provider.reasoningEffort}</small></td>
                  <td><span className={`state-pill state-${provider.hasApiKey ? "healthy" : "unavailable"}`}>{provider.hasApiKey ? "Keychain 已保存" : "缺少 Key"}</span></td>
                  <td>{running ? <span className="state-pill state-healthy">运行中</span> : <span className="state-pill state-disabled">待机</span>}</td>
                  <td><div className="gateway-row-actions"><button type="button" className="text-button" onClick={() => chooseProvider(provider)}>编辑</button><button type="button" className="text-button danger-text" onClick={() => void deleteProvider(provider)} disabled={running || busy !== null}>{busy === `delete:${provider.id}` ? "删除中…" : "删除"}</button></div></td>
                </tr>;
              })}
            </tbody></table></div>
          ) : <div className="empty-state"><strong>尚无第三方供应商</strong><p>新增一个 OpenAI Responses-compatible 上游后才能启动本地网关。</p></div>}
        </section>

        <form className="panel provider-form" onSubmit={(event) => void saveProvider(event)}>
          <div className="panel-title-row"><div><h2>{draft.id ? "编辑供应商" : "新增供应商"}</h2><p>{draft.id ? "API Key 留空会保留 Keychain 中的现有值。" : "初版只支持 Responses 协议。"}</p></div></div>
          {editingRunningProvider ? <p className="gateway-error" role="status">该供应商正在被网关使用。请先停止网关，再修改地址、模型或 API Key。</p> : null}
          <div className="provider-form-fields">
            <label><span>名称</span><input value={draft.name} maxLength={80} required onChange={(event) => setDraft({ ...draft, name: event.target.value })} placeholder="例如：公司 Responses 中转" /></label>
            <label><span>Base URL</span><input value={draft.baseUrl} required onChange={(event) => setDraft({ ...draft, baseUrl: event.target.value })} placeholder="https://api.example.com/v1" /></label>
            <label><span>模型</span><input value={draft.model} maxLength={128} required onChange={(event) => setDraft({ ...draft, model: event.target.value })} placeholder="gpt-5.6-sol" /></label>
            <label><span>推理等级</span><select value={draft.reasoningEffort} onChange={(event) => setDraft({ ...draft, reasoningEffort: event.target.value as CodexReasoningEffort })}>{efforts.map((effort) => <option key={effort} value={effort}>{effort}</option>)}</select></label>
            <label className="provider-key-field"><span>API Key</span><input type="password" autoComplete="off" value={draft.apiKey ?? ""} onChange={(event) => setDraft({ ...draft, apiKey: event.target.value })} placeholder={draft.id ? "留空以保留现有 Key" : "仅写入系统 Keychain"} /></label>
          </div>
          <div className="provider-form-actions"><button type="button" className="button button-secondary" onClick={createProvider}>清空</button><button type="submit" className="button button-primary" disabled={busy !== null || editingRunningProvider}>{busy === "save" ? "保存中…" : "保存供应商"}</button></div>
          <p className="provider-form-note">远程上游必须使用 HTTPS；仅 `localhost`、`127.0.0.1` 和 `::1` 可使用 HTTP。URL 中不允许用户名、密码、query 或 fragment。</p>
        </form>
      </div>
    </div>
  );
}
