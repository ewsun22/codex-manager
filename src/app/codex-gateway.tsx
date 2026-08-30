import { useCallback, useEffect, useMemo, useState, type FormEvent } from "react";
import { invokeBackend } from "./client.ts";
import {
  COMMANDS,
  type CodexGatewayStatus,
  type CodexGatewaySource,
  type CodexProviderProfile,
  type CodexReasoningEffort,
  type ProxyAuthImportOutcome,
  type ProxyAuthProfile,
  type ProxyAuthProfileList,
  type SaveCodexProviderInput,
} from "../shared/contracts.ts";
type GatewaySource = Exclude<CodexGatewaySource, "none">;

type GatewayNotice = (tone: "success" | "error" | "info", message: string) => void;
type GatewayModal = "provider" | "options" | null;

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
    "# API Key 始终省略；长期保存在 Keychain，启动时写入权限受限的内核运行配置。",
  ].join("\n");
}

/** A deliberately small dashboard control. It only exposes the loopback endpoint,
 * never the bearer or a provider API key. */
export function CodexGatewayQuickControl({ onOpenGateway }: { onOpenGateway: () => void }) {
  const [providers, setProviders] = useState<CodexProviderProfile[]>([]);
  const [oauthProfiles, setOauthProfiles] = useState<ProxyAuthProfile[]>([]);
  const [status, setStatus] = useState<CodexGatewayStatus | null>(null);
  const [selectedId, setSelectedId] = useState<string>("");
  const [source, setSource] = useState<GatewaySource>("oauth-pool");
  const [busy, setBusy] = useState<"install" | "start" | "stop" | null>(null);
  const [message, setMessage] = useState("正在读取本机代理状态…");

  const refresh = useCallback(async () => {
    try {
      const [nextProviders, nextAuth, nextStatus] = await Promise.all([
        invokeBackend<CodexProviderProfile[]>(COMMANDS.listCodexProviders),
        invokeBackend<ProxyAuthProfileList>(COMMANDS.listProxyAuthProfiles, { includeDeleted: false }),
        invokeBackend<CodexGatewayStatus>(COMMANDS.getCodexGatewayStatus),
      ]);
      setProviders(nextProviders);
      setOauthProfiles(nextAuth.profiles);
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
  const enabledOauthProfiles = oauthProfiles.filter((profile) => profile.enabled && profile.state === "ready");
  const selected = availableProviders.find((provider) => provider.id === selectedId) ?? null;
  const running = status?.state === "running";
  const runningProvider = providers.find((provider) => provider.id === status?.providerId) ?? null;
  const upstreamSummary = running
    ? status?.providerName ?? "未知上游"
    : source === "oauth-pool"
      ? `OAuth 凭据池 · 已启用 ${enabledOauthProfiles.length}`
      : selected?.name ?? "未选择";
  const endpoint = status ? status.endpoint ?? `http://127.0.0.1:${status.port}/v1` : null;
  const proxyOrigin = status ? `http://127.0.0.1:${status.port}` : null;
  const quickEndpoints: Array<[string, string | null]> = [
    ["OpenAI", endpoint],
    ["Claude", proxyOrigin],
    ["Gemini", proxyOrigin],
  ];
  const runGateway = async () => {
    if (busy || (source === "oauth-pool" ? !enabledOauthProfiles.length : !selected?.hasApiKey)) return;
    setMessage("");
    try {
      if (!status?.installed) {
        setBusy("install");
        setMessage("正在下载并校验官方 CLIProxyAPI 最新发行版…");
        const installed = await invokeBackend<CodexGatewayStatus>(COMMANDS.installLatestCliproxyCore);
        setStatus(installed);
      }
      setBusy("start");
      const next = await invokeBackend<CodexGatewayStatus>(COMMANDS.startCodexGateway, source === "oauth-pool"
        ? { source: "oauth-pool" }
        : { source: "external-provider", providerId: selected?.id });
      setStatus(next);
      setMessage(source === "oauth-pool" ? "CLIProxyAPI 反代已开启，使用本地 OAuth 凭据池。" : `CLIProxyAPI 反代已开启，上游为“${selected?.name ?? "外部供应商"}”。`);
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
      setMessage("CLIProxyAPI 反代已关闭。若 Codex 当前路由仍指向本地代理，请到“Codex 配置”恢复直连。");
    } catch (error) {
      setMessage(`代理未关闭：${errorMessage(error)}`);
    } finally {
      setBusy(null);
    }
  };
  const copyEndpoint = async (value: string | null, label: string) => {
    if (!value) {
      setMessage("代理状态尚未就绪，暂时不能复制 endpoint。");
      return;
    }
    try {
      await navigator.clipboard.writeText(value);
      setMessage(`${label} API URL 已复制。`);
    } catch {
      setMessage("系统拒绝写入剪贴板，请手动复制 endpoint。");
    }
  };

  return (
    <section className={`panel gateway-quick-panel${running ? " is-running" : ""}`} aria-labelledby="gateway-quick-heading" aria-busy={busy !== null}>
      <div className="panel-title-row gateway-quick-heading">
        <div>
          <p className="gateway-section-kicker">CLIPROXYAPI · LOOPBACK</p>
          <h2 id="gateway-quick-heading">代理 API 接入</h2>
          <p>预编译内核独立更新；支持 OpenAI、Claude 与 Gemini 兼容接入。</p>
        </div>
        <span className={`state-pill state-${running ? "healthy" : status?.state === "error" ? "degraded" : "disabled"}`}>{stateLabel(status)}</span>
      </div>
      <div className="gateway-quick-endpoints">
        {quickEndpoints.map(([label, value]) => <div className="gateway-quick-endpoint" key={label}>
          <span>{label}</span>
          <code title={value ?? undefined}>{value ?? "unavailable"}</code>
          <button type="button" className="button button-secondary" onClick={() => void copyEndpoint(value, label)} disabled={!value || busy !== null}>复制</button>
        </div>)}
      </div>
      <dl className="gateway-quick-meta">
        <div><dt>启动来源 / 模型</dt><dd>{upstreamSummary}{(running ? runningProvider : source === "external-provider" ? selected : null)?.model ? ` · ${(running ? runningProvider : selected)?.model}` : ""}</dd></div>
        <div><dt>CLIProxyAPI 内核</dt><dd>{status?.coreVersion ? `v${status.coreVersion.replace(/^v/, "")}` : status?.installed ? "版本 unavailable" : "未安装"}</dd></div>
        <div><dt>进程 PID</dt><dd>{status?.processId ?? "unavailable"}</dd></div>
      </dl>
      <div className="gateway-quick-actions">
        {!running ? <label><span>启动来源</span><select value={source} onChange={(event) => setSource(event.target.value as GatewaySource)} disabled={busy !== null}><option value="oauth-pool">OAuth 凭据池（{enabledOauthProfiles.length}）</option><option value="external-provider">外部 API Key 供应商</option></select></label> : null}
        {!running && source === "external-provider" && availableProviders.length ? <label><span>外部上游</span><select value={selectedId} onChange={(event) => setSelectedId(event.target.value)} disabled={busy !== null}>{availableProviders.map((provider) => <option key={provider.id} value={provider.id}>{provider.name}</option>)}</select></label> : null}
        {(enabledOauthProfiles.length || availableProviders.length) ? <button
          type="button"
          role="switch"
          aria-checked={running}
          className={`gateway-runtime-switch${running ? " is-on" : ""}`}
          onClick={() => void (running ? stopGateway() : runGateway())}
          disabled={!running && (source === "oauth-pool" ? !enabledOauthProfiles.length || busy !== null : !selected || busy !== null) || running && busy !== null}
        ><span aria-hidden="true" /><strong>{busy === "install" ? "安装内核中…" : busy === "start" ? "开启中…" : busy === "stop" ? "关闭中…" : running ? "反代已开启" : "开启反代"}</strong></button> : <button type="button" className="button button-primary" onClick={onOpenGateway}>配置本地代理</button>}
        <button type="button" className="button button-secondary" onClick={onOpenGateway} disabled={busy !== null}>本地代理</button>
      </div>
      {status?.lastError ? <p className="gateway-quick-error" role="alert">最近错误：{status.lastError}</p> : null}
      <p className="gateway-quick-message" aria-live="polite">{message}</p>
    </section>
  );
}

export function CodexGatewayView({
  onNotice,
}: {
  onNotice: GatewayNotice;
}) {
  const [providers, setProviders] = useState<CodexProviderProfile[]>([]);
  const [oauthProfiles, setOauthProfiles] = useState<ProxyAuthProfile[]>([]);
  const [deletedOauthProfiles, setDeletedOauthProfiles] = useState<ProxyAuthProfile[]>([]);
  const [status, setStatus] = useState<CodexGatewayStatus | null>(null);
  const [draft, setDraft] = useState<SaveCodexProviderInput>(emptyDraft);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [portDraft, setPortDraft] = useState("47653");
  const [modal, setModal] = useState<GatewayModal>(null);
  const [openMenuId, setOpenMenuId] = useState<string | null>(null);
  const [source, setSource] = useState<GatewaySource>("oauth-pool");
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState<string | null>(null);

  const selected = useMemo(
    () => providers.find((provider) => provider.id === selectedId) ?? null,
    [providers, selectedId],
  );
  const editingRunningProvider = status?.state === "running" && draft.id === status.providerId;
  const enabledOauthProfiles = oauthProfiles.filter((profile) => profile.enabled && profile.state === "ready");

  const refresh = useCallback(async () => {
    setLoading(true);
    try {
      const [nextProviders, nextAuth, nextAllAuth, nextStatus] = await Promise.all([
        invokeBackend<CodexProviderProfile[]>(COMMANDS.listCodexProviders),
        invokeBackend<ProxyAuthProfileList>(COMMANDS.listProxyAuthProfiles, { includeDeleted: false }),
        invokeBackend<ProxyAuthProfileList>(COMMANDS.listProxyAuthProfiles, { includeDeleted: true }),
        invokeBackend<CodexGatewayStatus>(COMMANDS.getCodexGatewayStatus),
      ]);
      setProviders(nextProviders);
      setOauthProfiles(nextAuth.profiles);
      setDeletedOauthProfiles(nextAllAuth.profiles.filter((profile) => !nextAuth.profiles.some((live) => live.id === profile.id)));
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
      setModal(null);
    };
  }, [refresh]);

  useEffect(() => {
    if (selected && !draft.id) setDraft(providerToDraft(selected));
  }, [draft.id, selected]);

  const chooseProvider = (provider: CodexProviderProfile) => {
    setSelectedId(provider.id);
    setDraft(providerToDraft(provider));
    setOpenMenuId(null);
  };

  const openCreateProvider = () => {
    setSelectedId(null);
    setDraft(emptyDraft);
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
      onNotice("success", `供应商“${saved.name}”已保存；API Key 长期保留在 Keychain，启动时写入权限受限的 CLIProxyAPI 运行配置。`);
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
      onNotice("success", `本地网关端口已设置为 ${next.port}。`);
    } catch (error) {
      onNotice("error", `端口未更新：${errorMessage(error)}`);
    } finally {
      setBusy(null);
    }
  };

  const checkCore = async () => {
    setBusy("core-check");
    try {
      const next = await invokeBackend<CodexGatewayStatus>(COMMANDS.checkLatestCliproxyCore);
      setStatus(next);
      onNotice("info", next.updateAvailable
        ? `CLIProxyAPI 可更新到 v${next.latestVersion?.replace(/^v/, "") ?? "unknown"}。`
        : next.installed ? "CLIProxyAPI 内核已是最新版。" : "已找到可安装的 CLIProxyAPI 最新版。");
    } catch (error) {
      onNotice("error", `CLIProxyAPI 版本检查失败：${errorMessage(error)}`);
    } finally {
      setBusy(null);
    }
  };

  const installCore = async () => {
    setBusy("core-install");
    try {
      const next = await invokeBackend<CodexGatewayStatus>(COMMANDS.installLatestCliproxyCore);
      setStatus(next);
      onNotice("success", `CLIProxyAPI v${next.coreVersion?.replace(/^v/, "") ?? "unknown"} 已通过 SHA-256 校验并安装。`);
    } catch (error) {
      onNotice("error", `CLIProxyAPI 内核未安装：${errorMessage(error)}`);
    } finally {
      setBusy(null);
    }
  };

  const startGateway = async (nextSource: GatewaySource, provider?: CodexProviderProfile) => {
    try {
      if (!status?.installed) {
        setBusy("core-install");
        const installed = await invokeBackend<CodexGatewayStatus>(COMMANDS.installLatestCliproxyCore);
        setStatus(installed);
      }
      setBusy("start");
      const next = await invokeBackend<CodexGatewayStatus>(COMMANDS.startCodexGateway, nextSource === "oauth-pool"
        ? { source: "oauth-pool" }
        : { source: "external-provider", providerId: provider?.id });
      setStatus(next);
      if (provider) {
        setSelectedId(provider.id);
        setDraft(providerToDraft(provider));
      }
      setOpenMenuId(null);
      onNotice("success", nextSource === "oauth-pool"
        ? "CLIProxyAPI 反代已启动，使用本地 OAuth 凭据池。"
        : `CLIProxyAPI 反代已启动，上游为“${provider?.name ?? "外部供应商"}”。`);
    } catch (error) {
      onNotice("error", `网关未启动：${errorMessage(error)}`);
    } finally {
      setBusy(null);
    }
  };

  const importOauthProfile = async () => {
    setBusy("oauth-import");
    try {
      const requestedLabel = window.prompt("为导入的 CLIProxyAPI OAuth 档案设置本地标签（不上传、不校验账户）：", "本地 OAuth 档案");
      if (requestedLabel === null) return;
      const outcome = await invokeBackend<ProxyAuthImportOutcome | null>(COMMANDS.importProxyAuthProfile, { label: requestedLabel.trim() });
      if (!outcome) {
        onNotice("info", "未选择认证文件，未作任何修改。");
        return;
      }
      await refresh();
      onNotice("success", "OAuth 认证档案已导入到独立本地 Keychain。身份仅用于本地防串号，不代表上游联网验真。");
    } catch (error) {
      onNotice("error", `OAuth 档案未导入：${errorMessage(error)}`);
    } finally {
      setBusy(null);
    }
  };

  const setOauthEnabled = async (profile: ProxyAuthProfile, enabled: boolean) => {
    setBusy(`oauth:${profile.id}`);
    try {
      await invokeBackend<ProxyAuthProfile>(COMMANDS.setProxyAuthProfileEnabled, { profileId: profile.id, enabled });
      await refresh();
      onNotice("success", `“${profile.label}”已${enabled ? "启用" : "禁用"}。`);
    } catch (error) {
      onNotice("error", `档案状态未更新：${errorMessage(error)}`);
    } finally {
      setBusy(null);
    }
  };

  const deleteOauthProfile = async (profile: ProxyAuthProfile) => {
    if (!window.confirm(`移除“${profile.label}”的本地代理 OAuth 档案？可在本页恢复。`)) return;
    setBusy(`oauth:${profile.id}`);
    try {
      await invokeBackend<ProxyAuthProfile>(COMMANDS.deleteProxyAuthProfile, { profileId: profile.id });
      await refresh();
      onNotice("success", `“${profile.label}”已移入本地代理档案回收区。`);
    } catch (error) {
      onNotice("error", `档案未移除：${errorMessage(error)}`);
    } finally {
      setBusy(null);
    }
  };

  const restoreOauthProfile = async (profile: ProxyAuthProfile) => {
    setBusy(`oauth:${profile.id}`);
    try {
      await invokeBackend<ProxyAuthProfile>(COMMANDS.restoreProxyAuthProfile, { profileId: profile.id });
      await refresh();
      onNotice("success", `“${profile.label}”已恢复；请按需启用。`);
    } catch (error) {
      onNotice("error", `档案未恢复：${errorMessage(error)}`);
    } finally {
      setBusy(null);
    }
  };

  const stopGateway = async () => {
    setBusy("stop");
    try {
      const next = await invokeBackend<CodexGatewayStatus>(COMMANDS.stopCodexGateway);
      setStatus(next);
      onNotice("success", "CLIProxyAPI 反代已停止。若 Codex 当前路由仍指向本地代理，请到“Codex 配置”恢复直连。");
    } catch (error) {
      onNotice("error", `网关未停止：${errorMessage(error)}`);
    } finally {
      setBusy(null);
    }
  };

  const testProvider = (provider: CodexProviderProfile) => {
    onNotice("info", `“${provider.name}”的模型测试尚未开放；为避免在未确认前发送请求，当前未连接上游。`);
  };

  const closeModal = () => {
    if (busy === "save") return;
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
          <h1>本地代理</h1>
          <p>使用独立更新的预编译 CLIProxyAPI 内核提供本机 loopback API；本页管理内核、代理 OAuth 凭据池和外部 API Key 上游，不改写 Codex 配置。</p>
        </div>
        <div className="gateway-heading-actions">
          <button type="button" className="button button-secondary" onClick={() => void refresh()} disabled={loading || busy !== null}>{loading ? "刷新中…" : "刷新"}</button>
          <button type="button" className="button button-secondary" onClick={() => setModal("options")} disabled={busy !== null}>更多选项</button>
        </div>
      </div>

      <section className={`panel cliproxy-core-panel${status?.updateAvailable ? " has-update" : ""}`} aria-labelledby="cliproxy-core-heading">
        <div className="gateway-provider-heading">
          <div>
            <p className="gateway-section-kicker">独立内核</p>
            <h2 id="cliproxy-core-heading">CLIProxyAPI Runtime</h2>
            <p>只从 router-for-me/CLIProxyAPI 官方 GitHub Release 下载当前系统与架构的预编译资产；Release digest 与 checksums.txt 必须一致。</p>
          </div>
          <span className={`state-pill state-${status?.state === "running" ? "healthy" : status?.installed ? status.updateAvailable ? "degraded" : "disabled" : "unavailable"}`}>{status?.state === "running" ? "运行中" : status?.installed ? status.updateAvailable ? "可更新" : "已安装" : "未安装"}</span>
        </div>
        <dl className="cliproxy-core-grid">
          <div><dt>当前版本</dt><dd>{status?.coreVersion ? `v${status.coreVersion.replace(/^v/, "")}` : "unavailable"}</dd></div>
          <div><dt>最新版本</dt><dd>{status?.latestVersion ? `v${status.latestVersion.replace(/^v/, "")}` : "尚未检查"}</dd></div>
          <div><dt>进程 PID</dt><dd>{status?.processId ?? "unavailable"}</dd></div>
          <div><dt>获取源</dt><dd>{status?.coreSource ?? "GitHub Release"}</dd></div>
        </dl>
        <div className="cliproxy-core-actions">
          <p>{status?.state === "running" ? "更新前需要先关闭反代；安装失败会保留旧内核。" : "下载、解压、切换都在应用专用目录中完成。"}</p>
          <button type="button" className="button button-secondary" onClick={() => void checkCore()} disabled={busy !== null}>{busy === "core-check" ? "检查中…" : "检查内核更新"}</button>
          {(!status?.installed || status.updateAvailable) ? <button type="button" className="button button-primary" onClick={() => void installCore()} disabled={status?.state === "running" || busy !== null}>{busy === "core-install" ? "安装中…" : status?.installed ? "更新到最新版" : "安装最新内核"}</button> : null}
        </div>
      </section>

      <section className="panel gateway-provider-panel" aria-labelledby="proxy-oauth-heading">
        <div className="gateway-provider-heading">
          <div>
            <p className="gateway-section-kicker">独立凭据域</p>
            <h2 id="proxy-oauth-heading">CLIProxyAPI OAuth 档案</h2>
            <p>认证文件仅由原生文件选择器导入，秘密保存在独立 Keychain。身份仅用于本地防串号，非上游联网验真；官方 Codex OAuth 不会进入这里。</p>
          </div>
          <button type="button" className="button button-primary" onClick={() => void importOauthProfile()} disabled={busy !== null}>导入认证文件</button>
        </div>
        <div className="proxy-auth-list">
          {loading && !oauthProfiles.length ? <div className="loading-state" role="status"><span className="loading-mark" aria-hidden="true" />正在读取本地 OAuth 档案…</div> : null}
          {!loading && !oauthProfiles.length ? <div className="empty-state proxy-auth-empty"><strong>还没有本地代理 OAuth 档案</strong><p>导入 CLIProxyAPI 兼容认证文件后，开启反代时才能选择 OAuth 凭据池。</p></div> : null}
          {oauthProfiles.map((profile) => <article key={profile.id} className="proxy-auth-row">
            <div><strong>{profile.label}</strong><small>{profile.provider} · 上次 checkpoint {profile.lastCheckpointAt ?? "unavailable"}</small></div>
            <span className={`state-pill state-${profile.enabled && profile.state === "ready" ? "healthy" : profile.state === "error" ? "degraded" : "disabled"}`}>{profile.enabled ? profile.state === "ready" ? "已启用" : profile.state : "已禁用"}</span>
            <div className="proxy-auth-actions">
              <button type="button" className="button button-secondary" onClick={() => void setOauthEnabled(profile, !profile.enabled)} disabled={busy !== null}>{profile.enabled ? "禁用" : "启用"}</button>
              <button type="button" className="button button-secondary" onClick={() => void deleteOauthProfile(profile)} disabled={busy !== null}>移除</button>
            </div>
          </article>)}
        </div>
        {deletedOauthProfiles.length ? <details className="proxy-auth-trash"><summary>已移除档案（{deletedOauthProfiles.length}）</summary>{deletedOauthProfiles.map((profile) => <div key={profile.id}><span>{profile.label} · {profile.provider}</span><button type="button" className="button button-secondary" onClick={() => void restoreOauthProfile(profile)} disabled={busy !== null}>恢复</button></div>)}</details> : null}
      </section>

      <section className="panel gateway-provider-panel" aria-labelledby="gateway-providers-heading">
        <div className="gateway-provider-heading">
          <div>
            <p className="gateway-section-kicker">外部上游</p>
            <h2 id="gateway-providers-heading">API Key 供应商</h2>
            <p>用于“外部 API Key 供应商”启动来源。API Key 长期保留在系统 Keychain；启动内核时按需写入权限受限的运行配置。</p>
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
                  <button type="button" className="button button-primary" onClick={() => void startGateway("external-provider", provider)} disabled={running || !provider.hasApiKey || busy !== null}>{running ? "运行中" : busy === "start" ? "启动中…" : !status?.installed ? "安装并启动" : "作为上游启动"}</button>
                  <button type="button" className="button button-secondary gateway-more-button" onClick={() => setOpenMenuId(hasOpenMenu ? null : provider.id)} aria-expanded={hasOpenMenu} aria-controls={`provider-menu-${provider.id}`} disabled={busy !== null}>更多</button>
                  {hasOpenMenu ? <div id={`provider-menu-${provider.id}`} className="gateway-card-menu" role="menu">
                    <button type="button" role="menuitem" onClick={() => openEditProvider(provider)}>编辑供应商</button>
                    <button type="button" role="menuitem" className="danger-text" onClick={() => void deleteProvider(provider)} disabled={running || busy !== null}>{busy === `delete:${provider.id}` ? "删除中…" : "删除供应商"}</button>
                  </div> : null}
                </div>
              </article>;
            })}
          </div>
        ) : <div className="empty-state gateway-provider-empty"><strong>还没有外部 API Key 供应商</strong><p>添加一个 OpenAI-compatible 上游，或使用上方 OAuth 凭据池启动本地代理。</p><button type="button" className="button button-primary" onClick={openCreateProvider}>添加供应商</button></div>}
      </section>

      <section className="panel gateway-start-source" aria-labelledby="gateway-start-source-heading">
        <div><p className="gateway-section-kicker">启动来源</p><h2 id="gateway-start-source-heading">选择本地代理上游</h2><p>运行状态与客户端 endpoint 不显示 bearer 或 API Key。</p></div>
        <div className="gateway-start-controls"><label><span>来源</span><select value={source} onChange={(event) => setSource(event.target.value as GatewaySource)} disabled={busy !== null}><option value="oauth-pool">OAuth 凭据池（已启用 {enabledOauthProfiles.length}）</option><option value="external-provider">外部 API Key 供应商</option></select></label>{source === "external-provider" ? <label><span>外部供应商</span><select value={selectedId ?? ""} onChange={(event) => setSelectedId(event.target.value)} disabled={busy !== null}>{providers.filter((provider) => provider.hasApiKey).map((provider) => <option key={provider.id} value={provider.id}>{provider.name}</option>)}</select></label> : null}<button type="button" className="button button-primary" onClick={() => void startGateway(source, source === "external-provider" ? providers.find((provider) => provider.id === selectedId) : undefined)} disabled={busy !== null || (source === "oauth-pool" ? !enabledOauthProfiles.length : !providers.some((provider) => provider.id === selectedId && provider.hasApiKey))}>{busy === "start" ? "启动中…" : status?.state === "running" ? "反代运行中" : "启动反代"}</button></div>
      </section>

      <details className={`panel gateway-status-panel gateway-status-${status?.state ?? "loading"}`} open={status?.state === "running"} aria-busy={loading}>
        <summary className="gateway-status-summary">
          <span><strong>CLIProxyAPI 本机反代</strong><small>{status?.endpoint ?? `http://127.0.0.1:${status?.port ?? portDraft}/v1`} · {status?.providerName ?? "未选择上游"}</small></span>
          <span className={`state-pill state-${status?.state === "running" ? "healthy" : status?.state === "error" ? "degraded" : "disabled"}`}>{stateLabel(status)}</span>
        </summary>
        <div className="gateway-status-grid">
          <div><span>监听地址</span><strong>{status?.endpoint ?? `http://127.0.0.1:${status?.port ?? portDraft}/v1`}</strong></div>
          <div><span>当前上游</span><strong>{status?.providerName ?? "未选择"}</strong></div>
          <div><span>内核版本</span><strong>{status?.coreVersion ? `v${status.coreVersion.replace(/^v/, "")}` : "unavailable"}</strong></div>
          <div><span>进程 PID</span><strong>{status?.processId ?? "unavailable"}</strong></div>
        </div>
        {status?.lastError ? <p className="gateway-error" role="alert">最近错误：{status.lastError}</p> : null}
        <div className="gateway-toolbar">
          <label><span>固定端口</span><input type="number" min="1024" max="65535" value={portDraft} onChange={(event) => setPortDraft(event.target.value)} disabled={status?.state === "running" || busy !== null} /></label>
          <button type="button" className="button button-secondary" onClick={() => void updatePort()} disabled={status?.state === "running" || busy !== null}>{busy === "port" ? "保存中…" : "保存端口"}</button>
          {status?.state === "running" ? (
            <>
              <button type="button" className="button button-danger" onClick={() => void stopGateway()} disabled={busy !== null}>{busy === "stop" ? "停止中…" : "停止网关"}</button>
            </>
          ) : <span className="gateway-toolbar-hint">在上方选择 OAuth 凭据池或外部 API Key 供应商后启动。</span>}
        </div>
      </details>

      <section className="gateway-capability-stack" aria-label="本地代理边界">
        <details className="gateway-capability">
          <summary><span>凭据与网络边界</span><small>默认关闭</small></summary>
          <div><p>本地代理 OAuth 与官方订阅完全隔离。启用 OAuth 凭据池仅表示将本地档案投影给 CLIProxyAPI；不会读取或复制官方 Codex OAuth，也不会把凭据状态当成上游联网验真。</p></div>
        </details>
      </section>

      {modal === "provider" ? (
        <div className="gateway-modal-backdrop" role="presentation" onMouseDown={closeModal}>
          <form className="gateway-modal provider-form" aria-labelledby="provider-modal-heading" aria-modal="true" role="dialog" onMouseDown={(event) => event.stopPropagation()} onSubmit={(event) => void saveProvider(event)}>
            <div className="gateway-modal-header">
              <div><p className="eyebrow">自定义供应商</p><h2 id="provider-modal-heading">{draft.id ? "编辑供应商" : "新增供应商"}</h2><p>{draft.id ? "API Key 留空会保留系统 Keychain 中的现有值。" : "当前供应商表单接入 OpenAI-compatible 上游。"}</p></div>
              <button type="button" className="gateway-modal-close" onClick={closeModal} aria-label="关闭供应商编辑">关闭</button>
            </div>
            {editingRunningProvider ? <p className="gateway-error" role="status">该供应商正在被网关使用。请先停止网关，再修改地址、模型或 API Key。</p> : null}
            <div className="gateway-modal-chips"><span>渠道：自定义</span><span>路由：CLIProxyAPI OpenAI compatibility</span></div>
            <div className="provider-form-fields">
              <label><span>名称</span><input value={draft.name} maxLength={80} required onChange={(event) => setDraft({ ...draft, name: event.target.value })} placeholder="例如：公司 OpenAI 中转" /></label>
              <label><span>Base URL</span><input value={draft.baseUrl} required onChange={(event) => setDraft({ ...draft, baseUrl: event.target.value })} placeholder="https://api.example.com/v1" /></label>
              <label><span>模型</span><input value={draft.model} maxLength={128} required onChange={(event) => setDraft({ ...draft, model: event.target.value })} placeholder="gpt-5.6-sol" /></label>
              <label><span>推理等级</span><select value={draft.reasoningEffort} onChange={(event) => setDraft({ ...draft, reasoningEffort: event.target.value as CodexReasoningEffort })}>{efforts.map((effort) => <option key={effort} value={effort}>{effort}</option>)}</select></label>
              <label className="provider-key-field"><span>API Key</span><input type="password" autoComplete="off" value={draft.apiKey ?? ""} onChange={(event) => setDraft({ ...draft, apiKey: event.target.value })} placeholder={draft.id ? "留空以保留现有 Key" : "仅写入系统 Keychain"} /></label>
            </div>
            <section className="gateway-preview" aria-labelledby="gateway-preview-heading"><div><h3 id="gateway-preview-heading">非敏感配置预览</h3><p>仅随表单变化；保存后也不会自动写入 Codex 文件。</p></div><pre>{nonSensitivePreview(draft)}</pre><p>本机访问凭据和上游 API Key 不会显示在这里；需要切换 Codex 路由时，请到“Codex 配置”预览并应用受管字段。</p></section>
            <div className="provider-form-actions"><button type="button" className="button button-secondary" onClick={() => setDraft(emptyDraft)} disabled={busy !== null}>清空</button><button type="submit" className="button button-primary" disabled={busy !== null || editingRunningProvider}>{busy === "save" ? "保存中…" : "保存供应商"}</button></div>
            <p className="provider-form-note">远程上游必须使用 HTTPS；仅 `localhost`、`127.0.0.1` 和 `::1` 可使用 HTTP。URL 中不允许用户名、密码、query 或 fragment。CLIProxyAPI 的 Claude、Gemini 与 OAuth 供应商配置尚未在本页开放。</p>
          </form>
        </div>
      ) : null}

      {modal === "options" ? (
        <div className="gateway-modal-backdrop" role="presentation" onMouseDown={closeModal}>
          <section className="gateway-modal gateway-options-modal" aria-labelledby="gateway-options-heading" aria-modal="true" role="dialog" onMouseDown={(event) => event.stopPropagation()}>
            <div className="gateway-modal-header"><div><p className="eyebrow">更多选项</p><h2 id="gateway-options-heading">配置能力预览</h2><p>下列开关尚未开放，均不会改变本机或 Codex 行为。</p></div><button type="button" className="gateway-modal-close" onClick={closeModal} aria-label="关闭更多选项">关闭</button></div>
            <div className="gateway-disabled-options">
              <label><span><strong>启停时自动写入 `config.toml`</strong><small>尚未开放；请在“Codex 配置”显式预览、应用或恢复路由。</small></span><input type="checkbox" disabled /></label>
              <label><span><strong>自动切换官方账户</strong><small>尚未开放；本页只管理独立的 CLIProxyAPI OAuth 档案，不访问官方订阅凭据。</small></span><input type="checkbox" disabled /></label>
              <label><span><strong>CLIProxyAPI 请求日志</strong><small>固定禁用；不会把请求或响应正文写入日志。</small></span><input type="checkbox" disabled /></label>
            </div>
          </section>
        </div>
      ) : null}

    </div>
  );
}
