import { useCallback, useEffect, useMemo, useState, type FormEvent } from "react";
import { invokeBackend } from "./client.ts";
import {
  COMMANDS,
  type CodexConfigApplyResult,
  type CodexConfigPreview,
  type CodexConfigProfile,
  type CodexConfigProfileKind,
  type CodexConfigSnapshot,
  type CodexProviderProfile,
  type CodexReasoningEffort,
  type SaveCodexProviderInput,
} from "../shared/contracts.ts";

type Notice = (tone: "success" | "error" | "info", message: string) => void;
type Draft = Omit<CodexConfigProfile, "id" | "isActive" | "isVerified" | "createdAt" | "updatedAt"> & { id?: string };
type ProviderDraft = SaveCodexProviderInput;
const efforts: CodexReasoningEffort[] = ["minimal", "low", "medium", "high", "xhigh"];
const emptyProviderDraft: ProviderDraft = { name: "", baseUrl: "", model: "", reasoningEffort: "high", apiKey: "" };

const kinds: Array<{ id: CodexConfigProfileKind; name: string; hint: string }> = [
  { id: "official-direct", name: "官方直连", hint: "恢复受信任的官方 Codex 配置；账户在“官方订阅”单独管理。" },
  { id: "local-cliproxy", name: "本地 CLIProxyAPI", hint: "把 Codex 路由到 loopback 本地代理，不复制 OAuth 凭据。" },
  { id: "external-compatible", name: "外部兼容代理", hint: "把 Codex 路由到一个 Responses 兼容 HTTP(S) 上游。" },
];

const emptyDraft = (kind: CodexConfigProfileKind = "local-cliproxy"): Draft => ({
  name: kind === "official-direct" ? "官方直连" : kind === "local-cliproxy" ? "本地 CLIProxyAPI" : "外部兼容代理",
  kind,
  baseUrl: kind === "local-cliproxy" ? "http://127.0.0.1:47653/v1" : "",
  model: "gpt-5.6-sol",
  reasoningEffort: "high",
  secretRef: kind === "local-cliproxy" ? "local-proxy-client" : "",
});

function errorText(error: unknown): string {
  return error instanceof Error ? error.message : "操作未完成，请刷新后重试。";
}

function kindLabel(kind: CodexConfigProfileKind): string {
  return kinds.find((item) => item.id === kind)?.name ?? kind;
}

export function CodexConfigView({ onNotice, onOpenOfficialSubscription }: { onNotice: Notice; onOpenOfficialSubscription: () => void }) {
  const [snapshot, setSnapshot] = useState<CodexConfigSnapshot | null>(null);
  const [providers, setProviders] = useState<CodexProviderProfile[]>([]);
  const [draft, setDraft] = useState<Draft | null>(null);
  const [providerDraft, setProviderDraft] = useState<ProviderDraft | null>(null);
  const [preview, setPreview] = useState<CodexConfigPreview | null>(null);
  const [busy, setBusy] = useState<string | null>(null);

  const active = useMemo(() => snapshot?.profiles.find((profile) => profile.id === snapshot.activeProfileId) ?? null, [snapshot]);

  const refresh = useCallback(async () => {
    setBusy("refresh");
    try {
      const [nextSnapshot, nextProviders] = await Promise.all([
        invokeBackend<CodexConfigSnapshot>(COMMANDS.getCodexConfigSnapshot),
        invokeBackend<CodexProviderProfile[]>(COMMANDS.listCodexProviders),
      ]);
      setSnapshot(nextSnapshot);
      setProviders(nextProviders);
    } catch (error) {
      onNotice("error", `Codex 配置状态不可用：${errorText(error)}`);
    } finally {
      setBusy(null);
    }
  }, [onNotice]);

  useEffect(() => { void refresh(); }, [refresh]);

  const externalProviders = providers.filter((provider) => provider.hasApiKey);

  const newDraft = (kind: CodexConfigProfileKind): Draft => {
    const next = emptyDraft(kind);
    if (kind === "external-compatible" && externalProviders[0]) {
      next.secretRef = `external-provider:${externalProviders[0].id}`;
      next.baseUrl = externalProviders[0].baseUrl;
      next.model = externalProviders[0].model;
      next.reasoningEffort = externalProviders[0].reasoningEffort;
    }
    if (kind === "local-cliproxy") {
      const local = snapshot?.profiles.find((profile) => profile.kind === "local-cliproxy");
      if (local?.baseUrl) next.baseUrl = local.baseUrl;
    }
    return next;
  };

  const editProfile = (profile: CodexConfigProfile) => {
    setDraft({
      id: profile.id,
      name: profile.name,
      kind: profile.kind,
      baseUrl: profile.baseUrl ?? "",
      model: profile.model ?? "",
      reasoningEffort: profile.reasoningEffort ?? "",
      secretRef: profile.secretRef ?? "",
    });
    setPreview(null);
  };

  const save = async (event: FormEvent) => {
    event.preventDefault();
    if (!draft) return;
    setBusy("save");
    try {
      const profile = await invokeBackend<CodexConfigProfile>(COMMANDS.saveCodexConfigProfile, {
        profile: {
          ...draft,
          id: draft.id ?? crypto.randomUUID(),
          name: draft.name.trim(),
          baseUrl: draft.baseUrl?.trim() || null,
          model: draft.model?.trim() || null,
          reasoningEffort: draft.reasoningEffort?.trim() || null,
          secretRef: draft.secretRef?.trim() || null,
          isActive: false,
          isVerified: false,
          createdAt: "",
          updatedAt: "",
        },
      });
      await refresh();
      editProfile(profile);
      onNotice("success", `配置档案“${profile.name}”已保存；尚未应用到 Codex。`);
    } catch (error) {
      onNotice("error", `配置档案未保存：${errorText(error)}`);
    } finally {
      setBusy(null);
    }
  };

  const showPreview = async (profile: CodexConfigProfile) => {
    setBusy(`preview:${profile.id}`);
    try {
      setPreview(await invokeBackend<CodexConfigPreview>(COMMANDS.previewCodexConfigProfile, { profileId: profile.id }));
    } catch (error) {
      onNotice("error", `无法生成预览：${errorText(error)}`);
    } finally {
      setBusy(null);
    }
  };

  const apply = async (profile: CodexConfigProfile) => {
    if (!window.confirm(`应用“${profile.name}”到 Codex 配置？应用只管理模型、provider 与对应 provider table；原配置会保存在本地事务日志中以供恢复。`)) return;
    setBusy(`apply:${profile.id}`);
    try {
      const result = await invokeBackend<CodexConfigApplyResult>(COMMANDS.applyCodexConfigProfile, { profileId: profile.id, expectedRevision: snapshot?.activeRevision ?? undefined });
      await refresh();
      onNotice(result.verified ? "success" : "info", result.message ?? `“${profile.name}”已应用。`);
    } catch (error) {
      onNotice("error", `配置未应用：${errorText(error)}`);
    } finally {
      setBusy(null);
    }
  };

  const restore = async () => {
    if (!window.confirm("恢复 Codex 配置到本工具接管前的状态？若外部已修改配置，恢复会要求你先刷新确认。")) return;
    setBusy("restore");
    try {
      const result = await invokeBackend<CodexConfigApplyResult>(COMMANDS.restoreCodexConfig, { expectedRevision: snapshot?.activeRevision ?? undefined });
      await refresh();
      setPreview(null);
      onNotice(result.verified ? "success" : "info", result.message ?? "Codex 配置已恢复。 ");
    } catch (error) {
      onNotice("error", `配置未恢复：${errorText(error)}`);
    } finally {
      setBusy(null);
    }
  };

  const remove = async (profile: CodexConfigProfile) => {
    if (!window.confirm(`删除配置档案“${profile.name}”？不会删除任何 Keychain 凭据。`)) return;
    setBusy(`remove:${profile.id}`);
    try {
      await invokeBackend<boolean>(COMMANDS.deleteCodexConfigProfile, { profileId: profile.id });
      setDraft(null);
      setPreview(null);
      await refresh();
      onNotice("success", `配置档案“${profile.name}”已删除。`);
    } catch (error) {
      onNotice("error", `配置档案未删除：${errorText(error)}`);
    } finally {
      setBusy(null);
    }
  };

  const editProvider = (provider: CodexProviderProfile) => {
    setProviderDraft({ id: provider.id, name: provider.name, baseUrl: provider.baseUrl, model: provider.model, reasoningEffort: provider.reasoningEffort, apiKey: "" });
    setDraft(null);
  };

  const saveProvider = async (event: FormEvent) => {
    event.preventDefault();
    if (!providerDraft) return;
    setBusy("save-provider");
    try {
      const saved = await invokeBackend<CodexProviderProfile>(COMMANDS.saveCodexProvider, {
        input: { ...providerDraft, name: providerDraft.name.trim(), baseUrl: providerDraft.baseUrl.trim(), model: providerDraft.model.trim(), apiKey: providerDraft.apiKey?.trim() || undefined },
      });
      await refresh();
      editProvider(saved);
      onNotice("success", `外部供应商“${saved.name}”已保存。Codex 将直接访问其 endpoint，不经过 CLIProxyAPI。`);
    } catch (error) {
      onNotice("error", `外部供应商未保存：${errorText(error)}`);
    } finally { setBusy(null); }
  };

  const deleteProvider = async (provider: CodexProviderProfile) => {
    if (!window.confirm(`删除外部供应商“${provider.name}”及其 Keychain API Key？此操作不可恢复。`)) return;
    setBusy(`delete-provider:${provider.id}`);
    try {
      await invokeBackend<CodexProviderProfile[]>(COMMANDS.deleteCodexProvider, { providerId: provider.id });
      if (providerDraft?.id === provider.id) setProviderDraft(null);
      await refresh();
      onNotice("success", `外部供应商“${provider.name}”已删除。`);
    } catch (error) {
      onNotice("error", `外部供应商未删除：${errorText(error)}`);
    } finally { setBusy(null); }
  };

  return <div className="view-content config-view">
    <div className="gateway-page-heading">
      <div className="gateway-heading-copy"><p className="eyebrow">CODEX ROUTING</p><h1>Codex 配置</h1><p>在官方直连、本地 CLIProxyAPI 与外部 Responses 兼容代理间切换。配置档案只保存端点引用和非敏感模型字段；官方 OAuth 账户始终在“官方订阅”单独管理。</p></div>
      <div className="gateway-heading-actions"><button type="button" className="button button-secondary" onClick={() => void refresh()} disabled={busy !== null}>{busy === "refresh" ? "刷新中…" : "刷新"}</button><button type="button" className="button button-secondary" onClick={() => void restore()} disabled={busy !== null || !snapshot?.activeProfileId}>{busy === "restore" ? "恢复中…" : "恢复"}</button></div>
    </div>

    <section className={`panel config-route-panel state-${snapshot?.state === "active" || snapshot?.state === "official-direct" ? "healthy" : snapshot?.state === "drifted" || snapshot?.state === "recovery-required" ? "degraded" : "disabled"}`} aria-live="polite">
      <div><span>当前路由</span><strong>{active ? `${kindLabel(active.kind)} · ${active.name}` : "未由 Codex Manager 接管"}</strong><small>{snapshot?.message ?? (active?.baseUrl ?? "未写入任何代理地址")}</small></div><span className="state-pill state-disabled">{snapshot?.state ?? "读取中"}</span>
    </section>

    <section className="config-kind-grid" aria-label="创建配置档案">
      {kinds.map((kind) => <article className="panel config-kind-card" key={kind.id}><div><p className="gateway-section-kicker">{kind.id}</p><h2>{kind.name}</h2><p>{kind.hint}</p></div><button type="button" className="button button-secondary" onClick={() => { setDraft(newDraft(kind.id)); setProviderDraft(null); setPreview(null); }} disabled={kind.id === "external-compatible" && externalProviders.length === 0}>新建档案</button></article>)}
    </section>

    <section className="panel config-profile-panel" aria-labelledby="config-profiles-heading"><div className="gateway-provider-heading"><div><p className="gateway-section-kicker">档案列表</p><h2 id="config-profiles-heading">可切换路由</h2><p>应用前可预览受管字段。检测到外部修改时会停止覆盖，避免误写用户配置。</p></div></div><div className="config-profile-list">{snapshot?.profiles.length ? snapshot.profiles.map((profile) => <article className={`config-profile-row${profile.isActive ? " is-active" : ""}`} key={profile.id}><div><strong>{profile.name}</strong><small>{kindLabel(profile.kind)} · {profile.baseUrl ?? "官方默认连接"}</small></div><span className={`state-pill state-${profile.isActive ? "healthy" : profile.isVerified ? "disabled" : "unavailable"}`}>{profile.isActive ? "当前" : profile.isVerified ? "文件已复核" : "未应用复核"}</span><div className="config-profile-actions"><button type="button" className="button button-secondary" onClick={() => void showPreview(profile)} disabled={busy !== null}>预览</button><button type="button" className="button button-primary" onClick={() => void apply(profile)} disabled={busy !== null || profile.isActive}>{busy === `apply:${profile.id}` ? "应用中…" : profile.isActive ? "当前路由" : "应用"}</button><button type="button" className="button button-secondary" onClick={() => editProfile(profile)} disabled={busy !== null}>编辑</button><button type="button" className="button button-secondary" onClick={() => void remove(profile)} disabled={busy !== null || profile.isActive}>删除</button></div></article>) : <div className="empty-state"><strong>还没有配置档案</strong><p>从上方选择一种路由创建档案。创建不会立即改写 Codex。</p></div>}</div></section>

    <section className="panel config-official-entry"><div><p className="gateway-section-kicker">官方身份</p><h2>官方 OAuth 账户与额度</h2><p>账户切换、登录和额度只在“官方订阅”工作台处理，不与本地 CLIProxyAPI OAuth 混用。</p></div><button type="button" className="button button-secondary" onClick={onOpenOfficialSubscription}>打开官方订阅</button></section>

    <section className="panel gateway-provider-panel" aria-labelledby="external-providers-heading"><div className="gateway-provider-heading"><div><p className="gateway-section-kicker">外部直连</p><h2 id="external-providers-heading">Responses 兼容供应商</h2><p>外部 API Key 保存在系统 Keychain。应用对应“外部兼容代理”档案后，Codex 直接访问该 endpoint，不经过 CLIProxyAPI。</p></div><button type="button" className="button button-primary" onClick={() => { setProviderDraft(emptyProviderDraft); setDraft(null); }} disabled={busy !== null}>新增供应商</button></div><div className="config-provider-list">{providers.length ? providers.map((provider) => <article className="config-profile-row" key={provider.id}><div><strong>{provider.name}</strong><small>{provider.baseUrl} · {provider.model} · {provider.reasoningEffort}</small></div><span className={`state-pill state-${provider.hasApiKey ? "disabled" : "unavailable"}`}>{provider.hasApiKey ? "Keychain 已保存" : "缺少 Key"}</span><div className="config-profile-actions"><button type="button" className="button button-secondary" onClick={() => editProvider(provider)} disabled={busy !== null}>编辑</button><button type="button" className="button button-secondary" onClick={() => void deleteProvider(provider)} disabled={busy !== null}>{busy === `delete-provider:${provider.id}` ? "删除中…" : "删除"}</button></div></article>) : <div className="empty-state"><strong>还没有外部供应商</strong><p>新增一个 Responses 兼容 endpoint 后，再创建外部兼容配置档案。</p></div>}</div></section>

    {draft ? <form className="panel config-editor" onSubmit={(event) => void save(event)}><div className="gateway-provider-heading"><div><p className="gateway-section-kicker">{draft.id ? "编辑档案" : "新建档案"}</p><h2>{kindLabel(draft.kind)}</h2><p>代理凭据只能从已保存的 Keychain 供应商或安装级本地凭据中选择，不允许手工输入 token。</p></div><button type="button" className="button button-secondary" onClick={() => { setDraft(null); setPreview(null); }}>关闭</button></div><div className="provider-form-fields"><label><span>名称</span><input value={draft.name} required maxLength={80} onChange={(event) => setDraft({ ...draft, name: event.target.value })} /></label><label><span>类型</span><select value={draft.kind} onChange={(event) => setDraft(newDraft(event.target.value as CodexConfigProfileKind))}>{kinds.map((kind) => <option value={kind.id} key={kind.id}>{kind.name}</option>)}</select></label>{draft.kind === "local-cliproxy" ? <label><span>Base URL</span><input value={draft.baseUrl ?? ""} readOnly aria-readonly="true" /></label> : null}{draft.kind === "external-compatible" ? <label><span>外部供应商</span><select value={draft.secretRef ?? ""} required onChange={(event) => { const provider = externalProviders.find((item) => `external-provider:${item.id}` === event.target.value); setDraft({ ...draft, secretRef: event.target.value, baseUrl: provider?.baseUrl ?? "", model: provider?.model ?? "", reasoningEffort: provider?.reasoningEffort ?? "" }); }}>{externalProviders.map((provider) => <option key={provider.id} value={`external-provider:${provider.id}`}>{provider.name} · {provider.baseUrl}</option>)}</select></label> : null}{draft.kind === "local-cliproxy" ? <><label><span>模型</span><input value={draft.model ?? ""} required onChange={(event) => setDraft({ ...draft, model: event.target.value })} placeholder="gpt-5.6-sol" /></label><label><span>推理等级</span><select value={draft.reasoningEffort ?? ""} onChange={(event) => setDraft({ ...draft, reasoningEffort: event.target.value })}><option value="">由 Codex 默认决定</option>{efforts.map((level) => <option key={level} value={level}>{level}</option>)}</select></label></> : null}</div><div className="provider-form-actions"><button type="submit" className="button button-primary" disabled={busy !== null || draft.kind === "external-compatible" && !draft.secretRef}>{busy === "save" ? "保存中…" : "保存档案"}</button></div></form> : null}

    {providerDraft ? <form className="panel config-editor" onSubmit={(event) => void saveProvider(event)}><div className="gateway-provider-heading"><div><p className="gateway-section-kicker">外部直连</p><h2>{providerDraft.id ? "编辑外部供应商" : "新增外部供应商"}</h2><p>API Key 只写入系统 Keychain。保存后必须创建并应用“外部兼容代理”档案，Codex 才会直接使用此 endpoint。</p></div><button type="button" className="button button-secondary" onClick={() => setProviderDraft(null)}>关闭</button></div><div className="provider-form-fields"><label><span>名称</span><input value={providerDraft.name} maxLength={80} required onChange={(event) => setProviderDraft({ ...providerDraft, name: event.target.value })} placeholder="例如：公司 OpenAI 中转" /></label><label><span>Base URL</span><input value={providerDraft.baseUrl} required onChange={(event) => setProviderDraft({ ...providerDraft, baseUrl: event.target.value })} placeholder="http://192.168.1.20:8317/v1 或 https://api.example.com/v1" /></label><label><span>模型</span><input value={providerDraft.model} maxLength={128} required onChange={(event) => setProviderDraft({ ...providerDraft, model: event.target.value })} placeholder="gpt-5.6-sol" /></label><label><span>推理等级</span><select value={providerDraft.reasoningEffort} onChange={(event) => setProviderDraft({ ...providerDraft, reasoningEffort: event.target.value as CodexReasoningEffort })}>{efforts.map((effort) => <option key={effort} value={effort}>{effort}</option>)}</select></label><label className="provider-key-field"><span>API Key</span><input type="password" autoComplete="off" value={providerDraft.apiKey ?? ""} onChange={(event) => setProviderDraft({ ...providerDraft, apiKey: event.target.value })} placeholder={providerDraft.id ? "留空以保留现有 Key" : "仅写入系统 Keychain"} /></label></div><p className="provider-form-note">支持 HTTP(S)，保存时不解析或访问该地址；公网 endpoint 建议使用 HTTPS。URL 中不允许用户名、密码、query 或 fragment。</p><div className="provider-form-actions"><button type="button" className="button button-secondary" onClick={() => setProviderDraft(emptyProviderDraft)} disabled={busy !== null}>清空</button><button type="submit" className="button button-primary" disabled={busy !== null}>{busy === "save-provider" ? "保存中…" : "保存供应商"}</button></div></form> : null}

    {preview ? <section className="panel config-preview-panel"><div className="gateway-provider-heading"><div><p className="gateway-section-kicker">应用预览</p><h2>受管配置字段</h2><p>{preview.managedFields.join(" · ")}</p></div><button type="button" className="button button-secondary" onClick={() => setPreview(null)}>关闭</button></div><pre>{preview.configSnippet}</pre></section> : null}
  </div>;
}
