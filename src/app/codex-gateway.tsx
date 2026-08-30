import { useCallback, useEffect, useState } from "react";
import { invokeBackend } from "./client.ts";
import {
  COMMANDS,
  type CodexGatewayStatus,
  type ProxyAuthImportOutcome,
  type ProxyAuthProfile,
  type ProxyAuthProfileList,
} from "../shared/contracts.ts";

type GatewayNotice = (tone: "success" | "error" | "info", message: string) => void;

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : "操作未完成，请刷新后重试。";
}

function stateLabel(status: CodexGatewayStatus | null): string {
  if (!status) return "正在读取";
  return { stopped: "已停止", running: "运行中", error: "异常" }[status.state];
}

/** 主页只管理本地 OAuth 凭据池启停；外部上游由 Codex 直接访问。 */
export function CodexGatewayQuickControl({ onOpenGateway }: { onOpenGateway: () => void }) {
  const [oauthProfiles, setOauthProfiles] = useState<ProxyAuthProfile[]>([]);
  const [status, setStatus] = useState<CodexGatewayStatus | null>(null);
  const [busy, setBusy] = useState<"install" | "start" | "stop" | null>(null);
  const [message, setMessage] = useState("正在读取本机代理状态…");

  const refresh = useCallback(async () => {
    try {
      const [nextAuth, nextStatus] = await Promise.all([
        invokeBackend<ProxyAuthProfileList>(COMMANDS.listProxyAuthProfiles, { includeDeleted: false }),
        invokeBackend<CodexGatewayStatus>(COMMANDS.getCodexGatewayStatus),
      ]);
      setOauthProfiles(nextAuth.profiles);
      setStatus(nextStatus);
      setMessage("");
    } catch (error) {
      setMessage(`代理状态不可用：${errorMessage(error)}`);
    }
  }, []);

  useEffect(() => { void refresh(); }, [refresh]);

  const enabledCount = oauthProfiles.filter((profile) => profile.enabled && profile.state === "ready").length;
  const running = status?.state === "running";
  const endpoint = status ? status.endpoint ?? `http://127.0.0.1:${status.port}/v1` : null;
  const proxyOrigin = status ? `http://127.0.0.1:${status.port}` : null;
  const quickEndpoints: Array<[string, string | null]> = [
    ["OpenAI", endpoint],
    ["Claude", proxyOrigin],
    ["Gemini", proxyOrigin],
  ];
  const runGateway = async () => {
    if (busy || !enabledCount) return;
    setMessage("");
    try {
      if (!status?.installed || status.updateAvailable) {
        setBusy("install");
        setMessage("正在下载并校验审核通过的 CLIProxyAPI 固定基线…");
        setStatus(await invokeBackend<CodexGatewayStatus>(COMMANDS.installLatestCliproxyCore));
      }
      setBusy("start");
      setStatus(await invokeBackend<CodexGatewayStatus>(COMMANDS.startCodexGateway));
      setMessage("CLIProxyAPI 反代已开启，使用本地 OAuth 凭据池。");
    } catch (error) {
      setMessage(`代理未开启：${errorMessage(error)}`);
    } finally {
      setBusy(null);
    }
  };
  const stopGateway = async () => {
    if (busy) return;
    setBusy("stop");
    try {
      setStatus(await invokeBackend<CodexGatewayStatus>(COMMANDS.stopCodexGateway));
      setMessage("CLIProxyAPI 反代已关闭。若 Codex 当前路由仍指向本地代理，请到“Codex 配置”恢复直连。");
    } catch (error) {
      setMessage(`代理未关闭：${errorMessage(error)}`);
    } finally {
      setBusy(null);
    }
  };
  const copyEndpoint = async (value: string | null, label: string) => {
    if (!value) return setMessage("代理状态尚未就绪，暂时不能复制 endpoint。");
    try {
      await navigator.clipboard.writeText(value);
      setMessage(`${label} API URL 已复制。`);
    } catch {
      setMessage("系统拒绝写入剪贴板，请手动复制 endpoint。");
    }
  };

  return <section className={`panel gateway-quick-panel${running ? " is-running" : ""}`} aria-labelledby="gateway-quick-heading" aria-busy={busy !== null}>
    <div className="panel-title-row gateway-quick-heading"><div><p className="gateway-section-kicker">CLIPROXYAPI · LOOPBACK</p><h2 id="gateway-quick-heading">代理 API 接入</h2><p>预编译内核独立版本、审核后更新；仅使用本机 OAuth 凭据池，支持 OpenAI、Claude 与 Gemini 兼容接入。</p></div><span className={`state-pill state-${running ? "healthy" : status?.state === "error" ? "degraded" : "disabled"}`}>{stateLabel(status)}</span></div>
    <div className="gateway-quick-endpoints">{quickEndpoints.map(([label, value]) => <div className="gateway-quick-endpoint" key={label}><span>{label}</span><code title={value ?? undefined}>{value ?? "unavailable"}</code><button type="button" className="button button-secondary" onClick={() => void copyEndpoint(value, label)} disabled={!value || busy !== null}>复制</button></div>)}</div>
    <dl className="gateway-quick-meta"><div><dt>OAuth 凭据池</dt><dd>已启用 {enabledCount}</dd></div><div><dt>CLIProxyAPI 内核</dt><dd>{status?.coreVersion ? `v${status.coreVersion.replace(/^v/, "")}` : status?.installed ? "版本 unavailable" : "未安装"}</dd></div><div><dt>进程 PID</dt><dd>{status?.processId ?? "unavailable"}</dd></div></dl>
    <div className="gateway-quick-actions">{enabledCount ? <button type="button" role="switch" aria-checked={running} className={`gateway-runtime-switch${running ? " is-on" : ""}`} onClick={() => void (running ? stopGateway() : runGateway())} disabled={busy !== null}><span aria-hidden="true" /><strong>{busy === "install" ? "安装内核中…" : busy === "start" ? "开启中…" : busy === "stop" ? "关闭中…" : running ? "反代已开启" : "开启反代"}</strong></button> : <button type="button" className="button button-primary" onClick={onOpenGateway}>导入 OAuth 档案</button>}<button type="button" className="button button-secondary" onClick={onOpenGateway} disabled={busy !== null}>本地代理</button></div>
    {status?.lastError ? <p className="gateway-quick-error" role="alert">最近错误：{status.lastError}</p> : null}<p className="gateway-quick-message" aria-live="polite">{message}</p>
  </section>;
}

export function CodexGatewayView({ onNotice }: { onNotice: GatewayNotice }) {
  const [oauthProfiles, setOauthProfiles] = useState<ProxyAuthProfile[]>([]);
  const [deletedOauthProfiles, setDeletedOauthProfiles] = useState<ProxyAuthProfile[]>([]);
  const [status, setStatus] = useState<CodexGatewayStatus | null>(null);
  const [portDraft, setPortDraft] = useState("47653");
  const [optionsOpen, setOptionsOpen] = useState(false);
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    setLoading(true);
    try {
      const [nextAuth, nextAllAuth, nextStatus] = await Promise.all([
        invokeBackend<ProxyAuthProfileList>(COMMANDS.listProxyAuthProfiles, { includeDeleted: false }),
        invokeBackend<ProxyAuthProfileList>(COMMANDS.listProxyAuthProfiles, { includeDeleted: true }),
        invokeBackend<CodexGatewayStatus>(COMMANDS.getCodexGatewayStatus),
      ]);
      setOauthProfiles(nextAuth.profiles);
      setDeletedOauthProfiles(nextAllAuth.profiles.filter((profile) => !nextAuth.profiles.some((live) => live.id === profile.id)));
      setStatus(nextStatus);
      setPortDraft(String(nextStatus.port));
    } catch (error) {
      onNotice("error", `本地代理状态不可用：${errorMessage(error)}`);
    } finally {
      setLoading(false);
    }
  }, [onNotice]);

  useEffect(() => { void refresh(); }, [refresh]);
  const enabledOauthProfiles = oauthProfiles.filter((profile) => profile.enabled && profile.state === "ready");

  const importOauthProfile = async () => {
    setBusy("oauth-import");
    try {
      const outcome = await invokeBackend<ProxyAuthImportOutcome>(COMMANDS.importProxyAuthProfile);
      await refresh();
      onNotice("success", outcome.action === "updated" ? `OAuth 档案“${outcome.profile.label}”已更新。` : `OAuth 档案“${outcome.profile.label}”已导入。`);
    } catch (error) {
      onNotice("error", `OAuth 档案未导入：${errorMessage(error)}`);
    } finally { setBusy(null); }
  };
  const setOauthEnabled = async (profile: ProxyAuthProfile, enabled: boolean) => {
    setBusy(`oauth:${profile.id}`);
    try {
      await invokeBackend<ProxyAuthProfile>(COMMANDS.setProxyAuthProfileEnabled, { profileId: profile.id, enabled });
      await refresh();
      onNotice("success", `OAuth 档案“${profile.label}”已${enabled ? "启用" : "禁用"}。`);
    } catch (error) { onNotice("error", `OAuth 档案未更新：${errorMessage(error)}`); } finally { setBusy(null); }
  };
  const deleteOauthProfile = async (profile: ProxyAuthProfile) => {
    if (!window.confirm(`移除 OAuth 档案“${profile.label}”？不会删除官方 Codex OAuth。`)) return;
    setBusy(`oauth-delete:${profile.id}`);
    try {
      await invokeBackend<ProxyAuthProfile>(COMMANDS.deleteProxyAuthProfile, { profileId: profile.id });
      await refresh();
      onNotice("success", `OAuth 档案“${profile.label}”已移除。`);
    } catch (error) { onNotice("error", `OAuth 档案未移除：${errorMessage(error)}`); } finally { setBusy(null); }
  };
  const restoreOauthProfile = async (profile: ProxyAuthProfile) => {
    setBusy(`oauth-restore:${profile.id}`);
    try {
      await invokeBackend<ProxyAuthProfile>(COMMANDS.restoreProxyAuthProfile, { profileId: profile.id });
      await refresh();
      onNotice("success", `OAuth 档案“${profile.label}”已恢复，需手动启用后才会进入凭据池。`);
    } catch (error) { onNotice("error", `OAuth 档案未恢复：${errorMessage(error)}`); } finally { setBusy(null); }
  };
  const updatePort = async () => {
    setBusy("port");
    try {
      const next = await invokeBackend<CodexGatewayStatus>(COMMANDS.updateCodexGatewayPort, { port: Number(portDraft) });
      setStatus(next); setPortDraft(String(next.port)); onNotice("success", `本地网关端口已设置为 ${next.port}。`);
    } catch (error) { onNotice("error", `端口未更新：${errorMessage(error)}`); } finally { setBusy(null); }
  };
  const checkCore = async () => {
    setBusy("core-check");
    try {
      const next = await invokeBackend<CodexGatewayStatus>(COMMANDS.checkLatestCliproxyCore); setStatus(next);
      onNotice("info", next.installed ? `已确认审核基线 v${next.latestVersion?.replace(/^v/, "") ?? "unknown"}。` : `已确认可安装的审核基线 v${next.latestVersion?.replace(/^v/, "") ?? "unknown"}。`);
    } catch (error) { onNotice("error", `CLIProxyAPI 版本检查失败：${errorMessage(error)}`); } finally { setBusy(null); }
  };
  const installCore = async () => {
    setBusy("core-install");
    try {
      const next = await invokeBackend<CodexGatewayStatus>(COMMANDS.installLatestCliproxyCore); setStatus(next);
      onNotice("success", `CLIProxyAPI v${next.coreVersion?.replace(/^v/, "") ?? "unknown"} 已校验并安装。`);
    } catch (error) { onNotice("error", `CLIProxyAPI 内核未安装：${errorMessage(error)}`); } finally { setBusy(null); }
  };
  const startGateway = async () => {
    if (!enabledOauthProfiles.length) return;
    try {
      if (!status?.installed || status.updateAvailable) { setBusy("core-install"); setStatus(await invokeBackend<CodexGatewayStatus>(COMMANDS.installLatestCliproxyCore)); }
      setBusy("start"); setStatus(await invokeBackend<CodexGatewayStatus>(COMMANDS.startCodexGateway));
      onNotice("success", "CLIProxyAPI 反代已启动，使用本地 OAuth 凭据池。");
    } catch (error) { onNotice("error", `反代未启动：${errorMessage(error)}`); } finally { setBusy(null); }
  };
  const stopGateway = async () => {
    setBusy("stop");
    try {
      setStatus(await invokeBackend<CodexGatewayStatus>(COMMANDS.stopCodexGateway));
      onNotice("info", "CLIProxyAPI 反代已停止。若 Codex 当前仍指向本地代理，请到“Codex 配置”恢复直连。");
    } catch (error) { onNotice("error", `网关未停止：${errorMessage(error)}`); } finally { setBusy(null); }
  };

  return <div className="view-content gateway-view">
    <div className="gateway-page-heading"><div className="gateway-heading-copy"><p className="eyebrow">CODEX MANAGER</p><h1>本地代理</h1><p>使用独立版本、审核后更新的预编译 CLIProxyAPI 内核提供本机 loopback API。本页只管理内核与独立 OAuth 凭据池；外部 Responses 供应商由“Codex 配置”保存并由 Codex 直接访问，不经过 CLIProxyAPI。</p></div><div className="gateway-heading-actions"><button type="button" className="button button-secondary" onClick={() => void refresh()} disabled={loading || busy !== null}>{loading ? "刷新中…" : "刷新"}</button><button type="button" className="button button-secondary" onClick={() => setOptionsOpen(true)} disabled={busy !== null}>更多选项</button></div></div>
    <section className="panel cliproxy-core-panel" aria-labelledby="cliproxy-core-heading"><div className="gateway-provider-heading"><div><p className="gateway-section-kicker">独立内核</p><h2 id="cliproxy-core-heading">CLIProxyAPI Runtime</h2><p>只安装已审核的 router-for-me/CLIProxyAPI 固定发行版资产；应用会校验平台、架构、文件名与 SHA-256，不自动追随上游最新版。</p></div><span className={`state-pill state-${status?.state === "running" ? "healthy" : status?.updateAvailable ? "degraded" : status?.installed ? "disabled" : "unavailable"}`}>{status?.state === "running" ? "运行中" : status?.updateAvailable ? "需切回基线" : status?.installed ? "已安装" : "未安装"}</span></div><dl className="cliproxy-core-grid"><div><dt>已安装版本</dt><dd>{status?.coreVersion ? `v${status.coreVersion.replace(/^v/, "")}` : "unavailable"}</dd></div><div><dt>审核基线</dt><dd>{status?.latestVersion ? `v${status.latestVersion.replace(/^v/, "")}` : "unavailable"}</dd></div><div><dt>进程 PID</dt><dd>{status?.processId ?? "unavailable"}</dd></div><div><dt>获取源</dt><dd>{status?.coreSource ?? "审核基线 unavailable"}</dd></div></dl><div className="cliproxy-core-actions"><p>{status?.state === "running" ? "重新安装前需要先关闭反代；安装失败会保留旧内核。" : status?.updateAvailable ? "已安装版本与审核基线不一致，启动前需切回基线。" : "下载、解压、切换都在应用专用目录中完成。"}</p><button type="button" className="button button-secondary" onClick={() => void checkCore()} disabled={busy !== null}>{busy === "core-check" ? "确认中…" : "确认审核基线"}</button>{!status?.installed || status.updateAvailable ? <button type="button" className="button button-primary" onClick={() => void installCore()} disabled={status?.state === "running" || busy !== null}>{busy === "core-install" ? "安装中…" : status?.installed ? "切回审核基线" : "安装审核基线"}</button> : null}</div></section>
    <section className="panel gateway-provider-panel" aria-labelledby="proxy-oauth-heading"><div className="gateway-provider-heading"><div><p className="gateway-section-kicker">独立凭据域</p><h2 id="proxy-oauth-heading">CLIProxyAPI OAuth 档案</h2><p>认证文件仅由原生文件选择器导入，秘密保存在独立 Keychain。身份仅用于本地防串号，非上游联网验真；官方 Codex OAuth 不会进入这里。</p></div><button type="button" className="button button-primary" onClick={() => void importOauthProfile()} disabled={busy !== null}>导入认证文件</button></div><div className="proxy-auth-list">{loading && !oauthProfiles.length ? <div className="loading-state" role="status"><span className="loading-mark" aria-hidden="true" />正在读取本地 OAuth 档案…</div> : null}{!loading && !oauthProfiles.length ? <div className="empty-state proxy-auth-empty"><strong>还没有本地代理 OAuth 档案</strong><p>导入 CLIProxyAPI 兼容认证文件后，才能开启 OAuth 凭据池。</p></div> : null}{oauthProfiles.map((profile) => <article key={profile.id} className="proxy-auth-row"><div><strong>{profile.label}</strong><small>{profile.provider} · 上次 checkpoint {profile.lastCheckpointAt ?? "unavailable"}</small></div><span className={`state-pill state-${profile.enabled && profile.state === "ready" ? "healthy" : profile.state === "error" ? "degraded" : "disabled"}`}>{profile.enabled ? profile.state === "ready" ? "已启用" : profile.state : "已禁用"}</span><div className="proxy-auth-actions"><button type="button" className="button button-secondary" onClick={() => void setOauthEnabled(profile, !profile.enabled)} disabled={busy !== null}>{profile.enabled ? "禁用" : "启用"}</button><button type="button" className="button button-secondary" onClick={() => void deleteOauthProfile(profile)} disabled={busy !== null}>移除</button></div></article>)}</div>{deletedOauthProfiles.length ? <details className="proxy-auth-trash"><summary>已移除档案（{deletedOauthProfiles.length}）</summary>{deletedOauthProfiles.map((profile) => <div key={profile.id}><span>{profile.label} · {profile.provider}</span><button type="button" className="button button-secondary" onClick={() => void restoreOauthProfile(profile)} disabled={busy !== null}>恢复</button></div>)}</details> : null}</section>
    <section className="panel gateway-start-source" aria-labelledby="gateway-start-source-heading"><div><p className="gateway-section-kicker">启动来源</p><h2 id="gateway-start-source-heading">OAuth 凭据池</h2><p>仅已启用且就绪的本地 OAuth 档案会投影给 CLIProxyAPI。外部 API Key 不经过本地代理。</p></div><div className="gateway-start-controls"><span className="state-pill state-disabled">已启用 {enabledOauthProfiles.length}</span><button type="button" className="button button-primary" onClick={() => void startGateway()} disabled={busy !== null || !enabledOauthProfiles.length || status?.state === "running"}>{busy === "start" || busy === "core-install" ? "启动中…" : status?.state === "running" ? "反代运行中" : "启动反代"}</button></div></section>
    <details className={`panel gateway-status-panel gateway-status-${status?.state ?? "loading"}`} open={status?.state === "running"} aria-busy={loading}><summary className="gateway-status-summary"><span><strong>CLIProxyAPI 本机反代</strong><small>{status?.endpoint ?? `http://127.0.0.1:${status?.port ?? portDraft}/v1`} · OAuth 凭据池</small></span><span className={`state-pill state-${status?.state === "running" ? "healthy" : status?.state === "error" ? "degraded" : "disabled"}`}>{stateLabel(status)}</span></summary><div className="gateway-status-grid"><div><span>监听地址</span><strong>{status?.endpoint ?? `http://127.0.0.1:${status?.port ?? portDraft}/v1`}</strong></div><div><span>当前上游</span><strong>CLIProxyAPI OAuth 凭据池</strong></div><div><span>内核版本</span><strong>{status?.coreVersion ? `v${status.coreVersion.replace(/^v/, "")}` : "unavailable"}</strong></div><div><span>进程 PID</span><strong>{status?.processId ?? "unavailable"}</strong></div></div>{status?.lastError ? <p className="gateway-error" role="alert">最近错误：{status.lastError}</p> : null}<div className="gateway-toolbar"><label><span>固定端口</span><input type="number" min="1024" max="65535" value={portDraft} onChange={(event) => setPortDraft(event.target.value)} disabled={status?.state === "running" || busy !== null} /></label><button type="button" className="button button-secondary" onClick={() => void updatePort()} disabled={status?.state === "running" || busy !== null}>{busy === "port" ? "保存中…" : "保存端口"}</button>{status?.state === "running" ? <button type="button" className="button button-danger" onClick={() => void stopGateway()} disabled={busy !== null}>{busy === "stop" ? "停止中…" : "停止网关"}</button> : <span className="gateway-toolbar-hint">启用至少一个 OAuth 档案后即可启动本地代理。</span>}</div></details>
    <section className="gateway-capability-stack" aria-label="本地代理边界"><details className="gateway-capability"><summary><span>凭据与网络边界</span><small>默认关闭</small></summary><div><p>本地代理 OAuth 与官方订阅完全隔离。启用凭据池只会将本地档案投影给 CLIProxyAPI；不会读取或复制官方 Codex OAuth，也不会把凭据状态当成上游联网验真。</p><p>外部 Responses 供应商在“Codex 配置”中保存，Codex 会直接访问其 endpoint，不经过 CLIProxyAPI。</p></div></details></section>
    {optionsOpen ? <div className="gateway-modal-backdrop" role="presentation" onMouseDown={() => setOptionsOpen(false)}><section className="gateway-modal gateway-options-modal" aria-labelledby="gateway-options-heading" aria-modal="true" role="dialog" onMouseDown={(event) => event.stopPropagation()}><div className="gateway-modal-header"><div><p className="eyebrow">更多选项</p><h2 id="gateway-options-heading">配置能力预览</h2><p>下列开关尚未开放，均不会改变本机或 Codex 行为。</p></div><button type="button" className="gateway-modal-close" onClick={() => setOptionsOpen(false)} aria-label="关闭更多选项">关闭</button></div><div className="gateway-disabled-options"><label><span><strong>启停时自动写入 config.toml</strong><small>尚未开放；请在“Codex 配置”显式预览、应用或恢复路由。</small></span><input type="checkbox" disabled /></label><label><span><strong>自动切换官方账户</strong><small>尚未开放；本页只管理独立的 CLIProxyAPI OAuth 档案，不访问官方订阅凭据。</small></span><input type="checkbox" disabled /></label><label><span><strong>CLIProxyAPI 请求日志</strong><small>固定禁用；不会把请求或响应正文写入日志。</small></span><input type="checkbox" disabled /></label></div></section></div> : null}
  </div>;
}
