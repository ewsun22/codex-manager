import { useCallback, useEffect, useMemo, useState, type FormEvent, type ReactNode } from "react";
import { invokeBackend, isDesktopRuntime } from "./app/client.ts";
import {
  confidenceLabel,
  formatCount,
  formatDate,
  formatMetric,
  formatMs,
  formatRatio,
  formatUsd,
} from "./app/format.ts";
import {
  COMMANDS,
  type ActivityPage,
  type ActivityQuery,
  type ActivityRecord,
  type AgentsChain,
  type AgentsFileSnapshot,
  type AgentsRevision,
  type AppSettings,
  type BootstrapPayload,
  type CreateAgentsFileInput,
  type MetricValue,
  type OtelConfig,
  type PricingRule,
  type ProjectSummary,
  type SourceHealth,
} from "./shared/contracts.ts";

type View = "overview" | "activity" | "projects" | "pricing" | "settings";

interface Notice {
  tone: "success" | "error" | "info";
  message: string;
}

const navItems: Array<{ id: View; label: string; helper: string }> = [
  { id: "overview", label: "总览", helper: "采集与使用概览" },
  { id: "activity", label: "活动记录", helper: "每次模型调用的元数据" },
  { id: "projects", label: "项目与 AGENTS", helper: "项目发现与安全编辑" },
  { id: "pricing", label: "价格目录", helper: "估算来源与覆盖规则" },
  { id: "settings", label: "设置", helper: "本地路径与保留策略" },
];

function statusLabel(state: SourceHealth["state"]): string {
  return {
    healthy: "健康",
    degraded: "需关注",
    disabled: "未启用",
    unavailable: "不可用",
  }[state];
}

function resultLabel(result: ActivityRecord["result"]): string {
  return {
    success: "成功",
    failure: "失败",
    running: "进行中",
    unknown: "未知",
  }[result];
}

function uniqueStrings(values: Array<string | null>): string[] {
  return [...new Set(values.filter((value): value is string => Boolean(value)))].sort();
}

function appError(error: unknown): string {
  return error instanceof Error ? error.message : "操作未完成，请查看本地采集状态后重试。";
}

function authorizedRootFor(path: string, roots: string[]): string | null {
  const normalizedPath = path.replace(/\\/g, "/");
  return roots
    .map((root) => root.replace(/\\/g, "/").replace(/\/$/, ""))
    .filter((root) => normalizedPath === root || normalizedPath.startsWith(`${root}/`))
    .sort((left, right) => right.length - left.length)[0] ?? null;
}

function ValueWithProvenance<T>({
  metric,
  format,
  className = "",
}: {
  metric: MetricValue<T>;
  format?: (value: T) => string;
  className?: string;
}) {
  const value = formatMetric(metric, format);
  const unavailable = metric.value === null;
  return (
    <span className={`metric-value ${unavailable ? "is-unavailable" : ""} ${className}`.trim()}>
      <span>{value}</span>
      <span className="metric-source" title={`来源：${metric.source}；可信度：${confidenceLabel(metric.confidence)}`}>
        {unavailable ? "不可用" : confidenceLabel(metric.confidence)}
      </span>
    </span>
  );
}

function SectionHeader({ title, subtitle, action }: { title: string; subtitle: string; action?: ReactNode }) {
  return (
    <div className="section-heading">
      <div>
        <p className="eyebrow">CODEX MANAGER</p>
        <h1>{title}</h1>
        <p>{subtitle}</p>
      </div>
      {action ? <div className="section-action">{action}</div> : null}
    </div>
  );
}

function InlineNotice({ notice, onClose }: { notice: Notice; onClose: () => void }) {
  return (
    <div className={`notice notice-${notice.tone}`} role={notice.tone === "error" ? "alert" : "status"}>
      <span>{notice.message}</span>
      <button type="button" className="text-button" onClick={onClose}>
        知道了
      </button>
    </div>
  );
}

function EmptyState({ title, detail, action }: { title: string; detail: string; action?: ReactNode }) {
  return (
    <div className="empty-state">
      <strong>{title}</strong>
      <p>{detail}</p>
      {action}
    </div>
  );
}

function LoadingState({ label = "正在读取本地元数据…" }: { label?: string }) {
  return (
    <div className="loading-state" role="status" aria-live="polite">
      <span className="loading-mark" aria-hidden="true" />
      {label}
    </div>
  );
}

function MetricCard({ label, value, detail, tone = "default" }: { label: string; value: string; detail: string; tone?: "default" | "accent" | "warn" }) {
  return (
    <article className={`metric-card metric-card-${tone}`}>
      <span>{label}</span>
      <strong>{value}</strong>
      <small>{detail}</small>
    </article>
  );
}

function SourceHealthCards({ sources }: { sources: SourceHealth[] }) {
  return (
    <section className="panel source-panel" aria-labelledby="source-health-heading">
      <div className="panel-title-row">
        <div>
          <h2 id="source-health-heading">来源健康</h2>
          <p>每个指标的采集来源和缺失状态均可追溯。</p>
        </div>
        <span className="count-chip">{sources.length} 个来源</span>
      </div>
      <div className="source-list">
        {sources.map((source) => (
          <article key={source.id} className="source-item">
            <div className="source-topline">
              <strong>{source.label}</strong>
              <span className={`state-pill state-${source.state}`}>{statusLabel(source.state)}</span>
            </div>
            <p>{source.message ?? "暂无额外说明。"}</p>
            <div className="source-meta">
              <span>最后观测：{formatDate(source.lastObservedAt)}</span>
              <span>滞后：{formatMs(source.lagMs)}</span>
              <span>未解析事件：{formatCount(source.unparsedEvents)}</span>
            </div>
          </article>
        ))}
      </div>
    </section>
  );
}

function Overview({ payload, onNavigate }: { payload: BootstrapPayload; onNavigate: (view: View) => void }) {
  const successRate = payload.summary.records > 0 ? payload.summary.successful / payload.summary.records : null;
  const cacheRatio =
    payload.summary.inputTokens !== null
    && payload.summary.inputTokens > 0
    && payload.summary.cachedInputTokens !== null
      ? payload.summary.cachedInputTokens / payload.summary.inputTokens
      : null;

  return (
    <div className="view-content">
      <SectionHeader
        title="本地 Codex 运行概览"
        subtitle={`${payload.summary.rangeLabel} · 仅保留元数据，不读取或保存消息正文。`}
        action={
          <button type="button" className="button button-primary" onClick={() => onNavigate("activity")}>
            查看活动记录
          </button>
        }
      />

      <section className="metric-grid" aria-label="核心指标">
        <MetricCard label="交互记录" value={formatCount(payload.summary.records)} detail={`${formatCount(payload.summary.successful)} 成功 · ${formatCount(payload.summary.failed)} 失败`} />
        <MetricCard label="缓存命中" value={formatRatio(cacheRatio)} detail={`${formatCount(payload.summary.cachedInputTokens, true)} read · ${formatCount(payload.summary.cacheWriteInputTokens, true)} write`} tone="accent" />
        <MetricCard label="API 等价估算" value={formatUsd(payload.summary.equivalentCostUsd)} detail="目录估算；套餐与中转账单不可得" tone="accent" />
        <MetricCard label="成功率" value={formatRatio(successRate)} detail="缺少状态码的记录不会猜测" tone={payload.summary.failed ? "warn" : "default"} />
      </section>

      <section className="overview-grid">
        <section className="panel overview-activity" aria-labelledby="recent-activity-heading">
          <div className="panel-title-row">
            <div>
              <h2 id="recent-activity-heading">最近活动</h2>
              <p>包含归一化模型交互与已观测 API 请求；token 快照已去重。</p>
            </div>
            <button type="button" className="text-button" onClick={() => onNavigate("activity")}>
              全部记录
            </button>
          </div>
          <div className="compact-list">
            {payload.activity.items.slice(0, 4).map((record) => (
              <button key={record.id} type="button" className="compact-row" onClick={() => onNavigate("activity")}>
                <span className={`result-dot result-${record.result}`} aria-label={resultLabel(record.result)} />
                <span className="compact-row-main">
                  <strong>{formatMetric(record.model)}</strong>
                  <small>{record.projectName ?? "未关联项目"}</small>
                </span>
                <span className="compact-row-metric">{formatMetric(record.totalTokens, formatCount)}</span>
                <time dateTime={record.occurredAt}>{formatDate(record.occurredAt)}</time>
              </button>
            ))}
          </div>
        </section>

        <section className="panel collection-panel" aria-labelledby="collection-heading">
          <div className="panel-title-row">
            <div>
              <h2 id="collection-heading">采集边界</h2>
              <p>默认隐私策略已生效。</p>
            </div>
          </div>
          <dl className="boundary-list">
            <div><dt>消息正文</dt><dd>不持久化</dd></div>
            <div><dt>凭证与请求查询</dt><dd>不持久化</dd></div>
            <div><dt>路径与项目名</dt><dd>本机 SQLite</dd></div>
            <div><dt>OTel receiver</dt><dd>{payload.settings.telemetryEnabled ? "已启用" : "未启用"}</dd></div>
          </dl>
          <button type="button" className="button button-secondary" onClick={() => onNavigate("settings")}>
            查看采集设置
          </button>
        </section>
      </section>

      <SourceHealthCards sources={payload.sources} />
    </div>
  );
}

function ActivityFilters({
  query,
  records,
  projects,
  onChange,
  onReset,
}: {
  query: ActivityQuery;
  records: ActivityRecord[];
  projects: ProjectSummary[];
  onChange: (next: ActivityQuery) => void;
  onReset: () => void;
}) {
  const models = uniqueStrings(records.map((record) => record.model.value));
  const efforts = uniqueStrings(records.map((record) => record.effort.value));
  return (
    <form className="filters" onSubmit={(event) => event.preventDefault()} aria-label="活动记录筛选">
      <label>
        <span>搜索</span>
        <input
          value={query.search ?? ""}
          placeholder="模型、会话、项目或 endpoint"
          onChange={(event) => onChange({ ...query, search: event.target.value || null, cursor: null })}
        />
      </label>
      <label>
        <span>项目</span>
        <select value={query.projectPath ?? ""} onChange={(event) => onChange({ ...query, projectPath: event.target.value || null, cursor: null })}>
          <option value="">全部项目</option>
          {projects.map((project) => <option key={project.id} value={project.canonicalPath}>{project.name}</option>)}
        </select>
      </label>
      <label>
        <span>模型</span>
        <select value={query.model ?? ""} onChange={(event) => onChange({ ...query, model: event.target.value || null, cursor: null })}>
          <option value="">全部模型</option>
          {models.map((model) => <option key={model} value={model}>{model}</option>)}
        </select>
      </label>
      <label>
        <span>推理级别</span>
        <select value={query.effort ?? ""} onChange={(event) => onChange({ ...query, effort: event.target.value || null, cursor: null })}>
          <option value="">全部级别</option>
          {efforts.map((effort) => <option key={effort} value={effort}>{effort}</option>)}
        </select>
      </label>
      <label>
        <span>结果</span>
        <select value={query.result ?? ""} onChange={(event) => onChange({ ...query, result: (event.target.value || null) as ActivityQuery["result"], cursor: null })}>
          <option value="">全部结果</option>
          <option value="success">成功</option>
          <option value="failure">失败</option>
          <option value="running">进行中</option>
          <option value="unknown">未知</option>
        </select>
      </label>
      <button type="button" className="text-button filter-reset" onClick={onReset}>清除筛选</button>
    </form>
  );
}

function ActivityTable({ page, onSelect }: { page: ActivityPage; onSelect: (record: ActivityRecord) => void }) {
  if (!page.items.length) {
    return <EmptyState title="没有匹配的活动记录" detail="调整筛选条件，或等待下一轮本地采集完成。" />;
  }
  return (
    <div className="table-scroll">
      <table className="activity-table">
        <thead>
          <tr>
            <th scope="col">时间</th>
            <th scope="col">模型 / 推理</th>
            <th scope="col">项目</th>
            <th scope="col">输入</th>
            <th scope="col">缓存读</th>
            <th scope="col">缓存写</th>
            <th scope="col">输出</th>
            <th scope="col">总计</th>
            <th scope="col">首个输出</th>
            <th scope="col">总耗时</th>
            <th scope="col">结果</th>
          </tr>
        </thead>
        <tbody>
          {page.items.map((record) => (
            <tr key={record.id} tabIndex={0} onClick={() => onSelect(record)} onKeyDown={(event) => {
              if (event.key === "Enter" || event.key === " ") onSelect(record);
            }}>
              <td><time dateTime={record.occurredAt}>{formatDate(record.occurredAt)}</time></td>
              <td><strong>{formatMetric(record.model)}</strong><small>{formatMetric(record.effort)}</small></td>
              <td><span className="project-cell">{record.projectName ?? "unavailable"}</span></td>
              <td><ValueWithProvenance metric={record.inputTokens} format={formatCount} /></td>
              <td><ValueWithProvenance metric={record.cachedInputTokens} format={formatCount} /></td>
              <td><ValueWithProvenance metric={record.cacheWriteInputTokens} format={formatCount} /></td>
              <td><ValueWithProvenance metric={record.outputTokens} format={formatCount} /></td>
              <td><ValueWithProvenance metric={record.totalTokens} format={formatCount} className="strong-value" /></td>
              <td><ValueWithProvenance metric={record.firstVisibleOutputMs} format={formatMs} /></td>
              <td><ValueWithProvenance metric={record.durationMs} format={formatMs} /></td>
              <td><span className={`result-pill result-${record.result}`}>{resultLabel(record.result)}{record.statusCode ? ` · ${record.statusCode}` : ""}</span></td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

function ActivityDrawer({ record, onClose }: { record: ActivityRecord; onClose: () => void }) {
  const items: Array<[string, MetricValue<string | number>, (value: string | number) => string]> = [
    ["模型", record.model, String],
    ["推理级别", record.effort, String],
    ["Provider", record.provider, String],
    ["请求地址", record.endpoint, String],
    ["输入 token", record.inputTokens, (value) => formatCount(Number(value))],
    ["缓存输入", record.cachedInputTokens, (value) => formatCount(Number(value))],
    ["缓存写入", record.cacheWriteInputTokens, (value) => formatCount(Number(value))],
    ["输出 token", record.outputTokens, (value) => formatCount(Number(value))],
    ["推理输出", record.reasoningOutputTokens, (value) => formatCount(Number(value))],
    ["总 token", record.totalTokens, (value) => formatCount(Number(value))],
    ["缓存命中率", record.cacheHitRatio, (value) => formatRatio(Number(value))],
    ["API 等价估算", record.equivalentCostUsd, (value) => formatUsd(Number(value))],
    ["首个输出", record.firstVisibleOutputMs, (value) => formatMs(Number(value))],
    ["全程耗时", record.durationMs, (value) => formatMs(Number(value))],
    ["响应字节", record.responseBytes, (value) => formatCount(Number(value))],
  ];
  return (
    <div className="drawer-backdrop" role="presentation" onMouseDown={onClose}>
      <aside className="drawer" role="dialog" aria-modal="true" aria-labelledby="activity-detail-title" onMouseDown={(event) => event.stopPropagation()}>
        <div className="drawer-header">
          <div>
            <p className="eyebrow">调用详情</p>
            <h2 id="activity-detail-title">{formatMetric(record.model)}</h2>
            <p>{formatDate(record.occurredAt)} · {record.projectName ?? "未关联项目"}</p>
          </div>
          <button type="button" className="button button-secondary" onClick={onClose}>关闭</button>
        </div>
        <div className="detail-callout">
          <span className={`result-pill result-${record.result}`}>{resultLabel(record.result)}</span>
          <span>会话 {record.sessionId}</span>
          <span>状态 {record.statusCode ?? "unavailable"}</span>
        </div>
        <dl className="detail-list">
          {items.map(([label, metric, format]) => (
            <div key={label}>
              <dt>{label}</dt>
              <dd><ValueWithProvenance metric={metric} format={format} /></dd>
            </div>
          ))}
        </dl>
        <p className="drawer-footnote">本页仅显示归一化后的元数据；请求正文、Authorization 与 Cookie 不会进入界面或数据库。</p>
      </aside>
    </div>
  );
}

function ActivityView({
  initial,
  projects,
  onLoad,
}: {
  initial: ActivityPage;
  projects: ProjectSummary[];
  onLoad: (query: ActivityQuery) => Promise<ActivityPage>;
}) {
  const [query, setQuery] = useState<ActivityQuery>({ limit: 50 });
  const [page, setPage] = useState<ActivityPage>(initial);
  const [selected, setSelected] = useState<ActivityRecord | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [cursorHistory, setCursorHistory] = useState<Array<string | null>>([]);

  const updateQuery = (next: ActivityQuery) => {
    setCursorHistory([]);
    setQuery({ ...next, cursor: null });
  };
  const goPrevious = () => {
    const previousCursor = cursorHistory.at(-1) ?? null;
    setCursorHistory((history) => history.slice(0, -1));
    setQuery((current) => ({ ...current, cursor: previousCursor }));
  };
  const goNext = () => {
    if (!page.nextCursor) return;
    setCursorHistory((history) => [...history, query.cursor ?? null]);
    setQuery((current) => ({ ...current, cursor: page.nextCursor }));
  };

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    setError(null);
    void onLoad(query)
      .then((next) => {
        if (!cancelled) setPage(next);
      })
      .catch((nextError: unknown) => {
        if (!cancelled) setError(appError(nextError));
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => { cancelled = true; };
  }, [query, onLoad]);

  return (
    <div className="view-content">
      <SectionHeader title="活动记录" subtitle="每行代表一次归一化模型交互或 OTel API 请求；没有模型调用的失败 turn 仍会保留。不能可靠取得的字段会明确显示 unavailable。" />
      <ActivityFilters
        query={query}
        records={initial.items}
        projects={projects}
        onChange={updateQuery}
        onReset={() => { setCursorHistory([]); setQuery({ limit: 50 }); }}
      />
      <section className="panel table-panel" aria-labelledby="activity-table-heading">
        <div className="panel-title-row">
          <div>
            <h2 id="activity-table-heading">{formatCount(page.total)} 条记录</h2>
            <p>可信度标签描述采集路径，不能替代账单或服务端日志。</p>
          </div>
          <label className="page-size">
            <span>每页</span>
            <select value={query.limit ?? 50} onChange={(event) => updateQuery({ ...query, limit: Number(event.target.value) })}>
              <option value={25}>25</option>
              <option value={50}>50</option>
              <option value={100}>100</option>
            </select>
          </label>
        </div>
        {error ? <InlineNotice notice={{ tone: "error", message: error }} onClose={() => setError(null)} /> : null}
        {loading ? <LoadingState label="正在应用筛选…" /> : <ActivityTable page={page} onSelect={setSelected} />}
        <div className="pagination" aria-label="活动记录分页">
          <button type="button" className="button button-secondary" onClick={goPrevious} disabled={loading || cursorHistory.length === 0}>上一页</button>
          <span>第 {cursorHistory.length + 1} 页 · 每页 {query.limit ?? 50} 条</span>
          <button type="button" className="button button-secondary" onClick={goNext} disabled={loading || !page.nextCursor}>下一页</button>
        </div>
      </section>
      {selected ? <ActivityDrawer record={selected} onClose={() => setSelected(null)} /> : null}
    </div>
  );
}

function ProjectList({
  projects,
  selectedPath,
  onSelect,
  onDiscover,
  discovering,
}: {
  projects: ProjectSummary[];
  selectedPath: string | null;
  onSelect: (project: ProjectSummary) => void;
  onDiscover: () => void;
  discovering: boolean;
}) {
  return (
    <section className="panel project-list-panel" aria-labelledby="projects-heading">
      <div className="panel-title-row">
        <div>
          <h2 id="projects-heading">已发现项目</h2>
          <p>来自本地观测路径和你授权的根目录。</p>
        </div>
        <button type="button" className="button button-secondary" onClick={onDiscover} disabled={discovering}>
          {discovering ? "扫描中…" : "重新发现"}
        </button>
      </div>
      {projects.length ? (
        <div className="project-list">
          {projects.map((project) => (
            <button
              type="button"
              key={project.id}
              className={`project-row ${selectedPath === project.canonicalPath ? "is-selected" : ""}`}
              onClick={() => onSelect(project)}
            >
              <span className="project-row-main">
                <strong>{project.name}</strong>
                <small>{project.canonicalPath}</small>
              </span>
              <span className="project-flags">
                {project.isGit ? <span>Git</span> : null}
                {project.worktree ? <span>Worktree</span> : null}
                <span>{project.agentsFileCount} 个 AGENTS</span>
              </span>
            </button>
          ))}
        </div>
      ) : <EmptyState title="尚未发现项目" detail="先在设置中授权一个项目根目录，再执行发现。" />}
    </section>
  );
}

function AgentsEditor({
  chain,
  snapshot,
  revisions,
  busy,
  canSave,
  canCreate,
  onOpen,
  onSave,
  onRestore,
  onCreate,
}: {
  chain: AgentsChain | null;
  snapshot: AgentsFileSnapshot | null;
  revisions: AgentsRevision[];
  busy: boolean;
  canSave: boolean;
  canCreate: boolean;
  onOpen: (path: string) => void;
  onSave: (content: string, snapshot: AgentsFileSnapshot) => void;
  onRestore: (revision: AgentsRevision) => void;
  onCreate: () => void;
}) {
  const [content, setContent] = useState("");
  const [baseHash, setBaseHash] = useState<string | null>(null);

  useEffect(() => {
    setContent(snapshot?.content ?? "");
    setBaseHash(snapshot?.sha256 ?? null);
  }, [snapshot]);

  if (!chain) {
    return <section className="panel agents-editor-panel"><EmptyState title="请选择一个项目" detail="选择项目后会解析当前工作目录的 AGENTS 层级。" /></section>;
  }

  const changed = Boolean(snapshot && baseHash === snapshot.sha256 && content !== snapshot.content);
  return (
    <section className="panel agents-editor-panel" aria-labelledby="agents-editor-heading">
      <div className="panel-title-row">
        <div>
          <h2 id="agents-editor-heading">AGENTS 层级与编辑器</h2>
          <p>有效文件由后端按 canonical path 与授权根目录确认；应用不会执行文件内容。</p>
        </div>
        {snapshot ? <span className={`state-pill ${snapshot.writable && canSave ? "state-healthy" : "state-unavailable"}`}>{snapshot.writable && canSave ? "可编辑" : "未授权或只读"}</span> : null}
      </div>

      <div className="chain-summary">
        <span>当前目录：<code>{chain.selectedCwd}</code></span>
        <span>有效规则：{chain.effectivePaths.length ? chain.effectivePaths.length : "unavailable"}</span>
        <span>单文件上限：{formatCount(chain.maxBytes)} B</span>
      </div>

      <div className="agents-layout">
        <div className="agents-file-list" role="list" aria-label="AGENTS 文件链">
          {chain.files.map((file) => (
            <button
              type="button"
              role="listitem"
              key={file.path}
              className={`agents-file-row ${snapshot?.path === file.path ? "is-open" : ""}`}
              onClick={() => onOpen(file.path)}
            >
              <strong>{file.relativePath}</strong>
              <small>{file.kind} · 优先级 {file.precedence}</small>
              <span>{file.effective ? "当前有效" : file.overridden ? "已被覆盖" : "候选"}</span>
            </button>
          ))}
        </div>
        <div className="editor-pane">
          {snapshot ? (
            <>
              <div className="editor-meta">
                <span><code>{snapshot.path}</code></span>
                <span>SHA {snapshot.sha256}</span>
                <span>{formatCount(snapshot.sizeBytes)} B</span>
              </div>
              <label className="sr-only" htmlFor="agents-content">AGENTS.md 内容</label>
              <textarea
                id="agents-content"
                className="agents-textarea"
                value={content}
                onChange={(event) => setContent(event.target.value)}
                spellCheck={false}
                readOnly={!snapshot.writable}
              />
              <div className="editor-actions">
                <span className={changed ? "changed-indicator" : "muted"}>{changed ? "存在未保存修改" : "已与磁盘快照一致"}</span>
                <button type="button" className="button button-secondary" onClick={() => setContent(snapshot.content)} disabled={!changed || busy}>放弃修改</button>
                <button type="button" className="button button-primary" onClick={() => onSave(content, snapshot)} disabled={!changed || !snapshot.writable || !canSave || busy}>{busy ? "保存中…" : "安全保存"}</button>
              </div>
            </>
          ) : (
            <EmptyState
              title="此项目没有项目级 AGENTS.md"
              detail={canCreate ? "可在已授权的项目根目录中显式创建 AGENTS.md；创建后才会进入安全编辑和 revision 流程。" : "文件不存在、位于未授权根目录、或未通过符号链接检查时，均不会显示为空白编辑器。"}
              action={canCreate ? <button type="button" className="button button-primary" onClick={onCreate} disabled={busy}>{busy ? "创建中…" : "创建项目级 AGENTS.md"}</button> : undefined}
            />
          )}
        </div>
      </div>

      {chain.warnings.length ? <div className="warning-list" role="status">{chain.warnings.map((warning) => <p key={warning}>{warning}</p>)}</div> : null}

      <div className="revision-section">
        <div className="panel-title-row">
          <div><h3>本地 revision</h3><p>仅列出由本应用安全保存时生成的版本。</p></div>
        </div>
        {revisions.length ? (
          <div className="revision-list">
            {revisions.map((revision) => (
              <div key={revision.id} className="revision-row">
                <span><strong>{formatDate(revision.createdAt)}</strong><small>{revision.afterSha256} · {formatCount(revision.byteLength)} B</small></span>
                <button type="button" className="text-button" onClick={() => onRestore(revision)} disabled={busy}>恢复此版本</button>
              </div>
            ))}
          </div>
        ) : <p className="muted">暂无由本应用创建的 revision。</p>}
      </div>
    </section>
  );
}

function ProjectsView({
  projects,
  authorizedRoots,
  onProjectsChange,
  showNotice,
}: {
  projects: ProjectSummary[];
  authorizedRoots: string[];
  onProjectsChange: (projects: ProjectSummary[]) => void;
  showNotice: (notice: Notice) => void;
}) {
  const [selectedProject, setSelectedProject] = useState<ProjectSummary | null>(projects[0] ?? null);
  const [chain, setChain] = useState<AgentsChain | null>(null);
  const [snapshot, setSnapshot] = useState<AgentsFileSnapshot | null>(null);
  const [revisions, setRevisions] = useState<AgentsRevision[]>([]);
  const [loading, setLoading] = useState(false);
  const [discovering, setDiscovering] = useState(false);

  const rootAgentsPath = selectedProject ? `${selectedProject.canonicalPath}/AGENTS.md` : null;
  const canCreate = Boolean(
    selectedProject
      && rootAgentsPath
      && authorizedRootFor(selectedProject.canonicalPath, authorizedRoots)
      && !chain?.files.some((file) => file.path === rootAgentsPath),
  );

  const loadChain = useCallback(async (project: ProjectSummary) => {
    setLoading(true);
    try {
      const nextChain = await invokeBackend<AgentsChain>(COMMANDS.getAgentsChain, { projectPath: project.canonicalPath, selectedCwd: project.canonicalPath });
      setChain(nextChain);
      const projectPrefix = `${project.canonicalPath.replace(/\/$/, "")}/`;
      const candidate = nextChain.files.find((file) => file.path.startsWith(projectPrefix) && file.effective && file.writable)
        ?? nextChain.files.find((file) => file.path.startsWith(projectPrefix) && file.writable);
      if (!candidate) {
        setSnapshot(null);
        setRevisions([]);
        return;
      }
      const [nextSnapshot, nextRevisions] = await Promise.all([
        invokeBackend<AgentsFileSnapshot>(COMMANDS.openAgentsFile, { path: candidate.path }),
        invokeBackend<AgentsRevision[]>(COMMANDS.listAgentsRevisions, { path: candidate.path }),
      ]);
      setSnapshot(nextSnapshot);
      setRevisions(nextRevisions);
    } catch (error: unknown) {
      showNotice({ tone: "error", message: `无法读取 AGENTS 层级：${appError(error)}` });
    } finally {
      setLoading(false);
    }
  }, [showNotice]);

  useEffect(() => {
    if (selectedProject) void loadChain(selectedProject);
  }, [loadChain, selectedProject]);

  const openFile = async (path: string) => {
    setLoading(true);
    try {
      const [nextSnapshot, nextRevisions] = await Promise.all([
        invokeBackend<AgentsFileSnapshot>(COMMANDS.openAgentsFile, { path }),
        invokeBackend<AgentsRevision[]>(COMMANDS.listAgentsRevisions, { path }),
      ]);
      setSnapshot(nextSnapshot);
      setRevisions(nextRevisions);
    } catch (error: unknown) {
      showNotice({ tone: "error", message: `无法打开文件：${appError(error)}` });
    } finally {
      setLoading(false);
    }
  };

  const save = async (content: string, current: AgentsFileSnapshot) => {
    if (!selectedProject) return;
    setLoading(true);
    try {
      const authorizedRoot = authorizedRootFor(current.path, authorizedRoots);
      if (!authorizedRoot) {
        showNotice({ tone: "error", message: "该文件不位于已授权根目录内，应用不会发送写入请求。" });
        return;
      }
      const result = await invokeBackend<{ snapshot: AgentsFileSnapshot; revisionId: string }>(COMMANDS.saveAgentsFile, {
        input: {
          path: current.path,
          authorizedRoot,
          content,
          expectedSha256: current.sha256,
          expectedMtimeMs: current.mtimeMs,
        },
      });
      setSnapshot(result.snapshot);
      setRevisions(await invokeBackend<AgentsRevision[]>(COMMANDS.listAgentsRevisions, { path: result.snapshot.path }));
      showNotice({ tone: "success", message: `已原子保存，并创建 revision ${result.revisionId}。` });
    } catch (error: unknown) {
      showNotice({ tone: "error", message: `保存被拒绝或发生外部冲突：${appError(error)}` });
    } finally {
      setLoading(false);
    }
  };

  const createProjectAgents = async () => {
    if (!selectedProject) return;
    const authorizedRoot = authorizedRootFor(selectedProject.canonicalPath, authorizedRoots);
    if (!authorizedRoot) {
      showNotice({ tone: "error", message: "项目根目录不在授权范围内，应用不会创建文件。" });
      return;
    }
    const confirmed = window.confirm(`将在项目根目录创建 AGENTS.md：\n${selectedProject.canonicalPath}\n\n继续吗？`);
    if (!confirmed) return;
    setLoading(true);
    try {
      const input: CreateAgentsFileInput = {
        projectPath: selectedProject.canonicalPath,
        authorizedRoot,
        content: "# 项目 AGENTS 说明\n\n## 工作边界\n\n- 在此处维护该项目的 Codex 协作规则。\n- 修改后请运行相关测试并报告实际结果。\n",
        fileName: "AGENTS.md",
      };
      const result = await invokeBackend<{ snapshot: AgentsFileSnapshot; revisionId: string }>(COMMANDS.createAgentsFile, { input });
      setSnapshot(result.snapshot);
      setRevisions(await invokeBackend<AgentsRevision[]>(COMMANDS.listAgentsRevisions, { path: result.snapshot.path }));
      await loadChain(selectedProject);
      showNotice({ tone: "success", message: `已创建项目级 AGENTS.md，并生成 revision ${result.revisionId}。` });
    } catch (error: unknown) {
      showNotice({ tone: "error", message: `创建被拒绝或未完成：${appError(error)}` });
    } finally {
      setLoading(false);
    }
  };

  const restore = async (revision: AgentsRevision) => {
    if (!snapshot) return;
    const confirmed = window.confirm(`恢复 ${formatDate(revision.createdAt)} 的 revision？当前磁盘内容会先由后端进行冲突校验。`);
    if (!confirmed) return;
    setLoading(true);
    try {
      const result = await invokeBackend<{ snapshot: AgentsFileSnapshot; revisionId: string }>(COMMANDS.restoreAgentsRevision, {
        revisionId: revision.id,
        expectedSha256: snapshot.sha256,
      });
      setSnapshot(result.snapshot);
      setRevisions(await invokeBackend<AgentsRevision[]>(COMMANDS.listAgentsRevisions, { path: result.snapshot.path }));
      showNotice({ tone: "success", message: `已恢复 revision，并创建 ${result.revisionId}。` });
    } catch (error: unknown) {
      showNotice({ tone: "error", message: `恢复被拒绝或发生外部冲突：${appError(error)}` });
    } finally {
      setLoading(false);
    }
  };

  const discover = async () => {
    setDiscovering(true);
    try {
      const next = await invokeBackend<ProjectSummary[]>(COMMANDS.discoverProjects);
      onProjectsChange(next);
      const current = next.find((project) => project.canonicalPath === selectedProject?.canonicalPath) ?? next[0] ?? null;
      setSelectedProject(current);
      showNotice({ tone: "success", message: `项目发现完成：${formatCount(next.length)} 个项目。` });
    } catch (error: unknown) {
      showNotice({ tone: "error", message: `项目发现未完成：${appError(error)}` });
    } finally {
      setDiscovering(false);
    }
  };

  return (
    <div className="view-content">
      <SectionHeader title="项目与 AGENTS" subtitle="查看所有已观测项目，并在授权根目录内安全编辑 AGENTS.md。" />
      <div className="projects-grid">
        <ProjectList projects={projects} selectedPath={selectedProject?.canonicalPath ?? null} onSelect={setSelectedProject} onDiscover={discover} discovering={discovering} />
        {loading && !chain ? <LoadingState label="正在解析项目层级…" /> : <AgentsEditor chain={chain} snapshot={snapshot} revisions={revisions} busy={loading} canSave={Boolean(snapshot && authorizedRootFor(snapshot.path, authorizedRoots))} canCreate={canCreate} onOpen={(path) => void openFile(path)} onSave={(content, current) => void save(content, current)} onRestore={(revision) => void restore(revision)} onCreate={() => void createProjectAgents()} />}
      </div>
    </div>
  );
}

function PricingView({ rules }: { rules: PricingRule[] }) {
  return (
    <div className="view-content">
      <SectionHeader title="价格目录" subtitle="价格只用于 API 等价估算；ChatGPT 套餐与第三方中转站真实账单均显示 unavailable。" />
      <section className="panel table-panel" aria-labelledby="pricing-heading">
        <div className="panel-title-row">
          <div><h2 id="pricing-heading">生效规则</h2><p>每条 API 等价估算都保留 catalog 生效日期与来源。</p></div>
          <span className="count-chip">{rules.length} 条规则</span>
        </div>
        {rules.length ? (
          <div className="table-scroll"><table className="pricing-table"><thead><tr><th>Provider</th><th>模型匹配</th><th>输入 / M</th><th>缓存读 / M</th><th>缓存写 / M</th><th>输出 / M</th><th>生效日期</th><th>来源</th></tr></thead><tbody>
            {rules.map((rule) => <tr key={rule.id}><td>{rule.provider}</td><td><code>{rule.modelPattern}</code></td><td>{formatUsd(rule.inputPerMillion)}</td><td>{formatUsd(rule.cachedInputPerMillion)}</td><td>{formatUsd(rule.cacheWriteInputPerMillion)}</td><td>{formatUsd(rule.outputPerMillion)}</td><td>{rule.effectiveFrom}</td><td>{rule.isUserOverride ? "用户覆盖" : rule.sourceLabel}</td></tr>)}
          </tbody></table></div>
        ) : <EmptyState title="价格目录不可用" detail="未匹配到规则时，调用详情会显示 unavailable 而不是显示零成本。" />}
      </section>
    </div>
  );
}

function SettingsView({
  settings,
  capability,
  onSave,
  onProbe,
  saving,
}: {
  settings: AppSettings;
  capability: BootstrapPayload["capability"];
  onSave: (next: AppSettings) => void;
  onProbe: () => void;
  saving: boolean;
}) {
  const [draft, setDraft] = useState(settings);
  const [otelConfig, setOtelConfig] = useState<OtelConfig | null>(null);
  const [otelConfigError, setOtelConfigError] = useState<string | null>(null);
  useEffect(() => setDraft(settings), [settings]);
  const submit = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    onSave(draft);
  };
  const revealOtelConfig = async () => {
    setOtelConfigError(null);
    try {
      setOtelConfig(await invokeBackend<OtelConfig>(COMMANDS.getOtelConfig));
    } catch (error: unknown) {
      setOtelConfig(null);
      setOtelConfigError(appError(error));
    }
  };
  return (
    <div className="view-content">
      <SectionHeader title="设置" subtitle="所有路径设置仅在本机使用。授权根目录之外的 AGENTS 文件写入会被后端拒绝。" />
      <div className="settings-grid">
        <form className="panel settings-form" onSubmit={submit}>
          <div className="panel-title-row"><div><h2>本地采集</h2><p>读取本机 Codex 产生的受支持元数据来源。</p></div></div>
          <label><span>Codex home（每行一个）</span><textarea value={draft.codexHomes.join("\n")} onChange={(event) => setDraft({ ...draft, codexHomes: event.target.value.split("\n").map((value) => value.trim()).filter(Boolean) })} /></label>
          <label><span>授权项目根目录（每行一个）</span><textarea value={draft.authorizedRoots.join("\n")} onChange={(event) => setDraft({ ...draft, authorizedRoots: event.target.value.split("\n").map((value) => value.trim()).filter(Boolean) })} /></label>
          <div className="settings-inline"><label><span>保留天数</span><input type="number" min="1" max="3650" value={draft.retentionDays} onChange={(event) => setDraft({ ...draft, retentionDays: Math.max(1, Number(event.target.value)) })} /></label><label className="checkbox-row"><input type="checkbox" checked={draft.telemetryEnabled} onChange={(event) => setDraft({ ...draft, telemetryEnabled: event.target.checked })} /><span>启用本机 OTel receiver</span></label></div>
          <p className="settings-disclaimer">此开关只控制本应用的 loopback receiver；它不会修改 Codex 配置。要产生 OTel 数据，需由你自行在 Codex 配置中明确启用并指向本机接收器。</p>
          <div className="otel-config-box">
            <div className="editor-actions">
              <span className="muted">配置片段包含本次运行的专用 token，只在你点击后显示。</span>
              <button className="button button-secondary" type="button" onClick={() => void revealOtelConfig()} disabled={!settings.telemetryEnabled}>显示 OTel 配置</button>
            </div>
            {otelConfig ? <><dl className="capability-list"><div><dt>Endpoint</dt><dd><code>{otelConfig.endpoint}</code></dd></div><div><dt>CA certificate</dt><dd><code>{otelConfig.certificatePath}</code></dd></div><div><dt>Auth header</dt><dd><code>{otelConfig.headerName}</code></dd></div></dl><textarea className="otel-config-snippet" readOnly spellCheck={false} value={otelConfig.configSnippet} aria-label="Codex OTel 配置片段" /></> : null}
            {otelConfigError ? <p className="capability-message">{otelConfigError}</p> : null}
          </div>
          <div className="privacy-lock"><strong>隐私锁定</strong><span>metadataOnly = true，消息正文、Authorization、Cookie、API Key 与完整环境变量不会保存。</span></div>
          <div className="editor-actions"><span className="muted">价格目录版本：{draft.priceCatalogVersion}</span><button className="button button-primary" type="submit" disabled={saving}>{saving ? "保存中…" : "保存设置"}</button></div>
        </form>
        <section className="panel capability-panel" aria-labelledby="capability-heading">
          <div className="panel-title-row"><div><h2 id="capability-heading">Codex capability</h2><p>仅探测 CLI schema，不会接管私有 app-server 进程。</p></div><button type="button" className="button button-secondary" onClick={onProbe} disabled={saving}>{saving ? "探测中…" : "重新探测"}</button></div>
          {capability ? <dl className="capability-list"><div><dt>可用性</dt><dd>{capability.available ? "可用" : "unavailable"}</dd></div><div><dt>版本</dt><dd>{capability.version ?? "unavailable"}</dd></div><div><dt>Schema SHA-256</dt><dd><code>{capability.schemaSha256 ?? "unavailable"}</code></dd></div><div><dt>上次探测</dt><dd>{formatDate(capability.checkedAt)}</dd></div><div><dt>可执行文件</dt><dd><code>{capability.executablePath}</code></dd></div></dl> : <EmptyState title="尚未探测 capability" detail="运行探测前不会将本机 Codex 数据格式描述为稳定 API。" />}
          {capability?.message ? <p className="capability-message">{capability.message}</p> : null}
        </section>
      </div>
    </div>
  );
}

export function App() {
  const [view, setView] = useState<View>("overview");
  const [payload, setPayload] = useState<BootstrapPayload | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<Notice | null>(null);
  const [saving, setSaving] = useState(false);

  const refreshBootstrap = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      setPayload(await invokeBackend<BootstrapPayload>(COMMANDS.bootstrap));
    } catch (nextError: unknown) {
      setError(appError(nextError));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => { void refreshBootstrap(); }, [refreshBootstrap]);

  const loadActivity = useCallback((query: ActivityQuery) => invokeBackend<ActivityPage>(COMMANDS.listActivity, { query }), []);
  const updateSettings = async (next: AppSettings) => {
    setSaving(true);
    try {
      const saved = await invokeBackend<AppSettings>(COMMANDS.updateSettings, { settings: next });
      const sources = await invokeBackend<SourceHealth[]>(COMMANDS.listSources);
      setPayload((current) => current ? { ...current, settings: saved, sources } : current);
      setNotice({ tone: "success", message: "设置已保存到本机，来源健康状态已刷新。" });
    } catch (nextError: unknown) {
      setNotice({ tone: "error", message: `设置未保存：${appError(nextError)}` });
    } finally {
      setSaving(false);
    }
  };
  const probe = async () => {
    setSaving(true);
    try {
      const capability = await invokeBackend<BootstrapPayload["capability"]>(COMMANDS.probeCodex);
      setPayload((current) => current ? { ...current, capability } : current);
      setNotice({ tone: "success", message: "Codex capability 探测完成。" });
    } catch (nextError: unknown) {
      setNotice({ tone: "error", message: `探测未完成：${appError(nextError)}` });
    } finally {
      setSaving(false);
    }
  };

  const shellLabel = isDesktopRuntime() ? "本地桌面模式" : "浏览器演示模式";
  const projects = payload?.projects ?? [];
  const current = useMemo(() => {
    if (!payload) return null;
    switch (view) {
      case "overview": return <Overview payload={payload} onNavigate={setView} />;
      case "activity": return <ActivityView initial={payload.activity} projects={projects} onLoad={loadActivity} />;
      case "projects": return <ProjectsView projects={projects} authorizedRoots={payload.settings.authorizedRoots} onProjectsChange={(next) => setPayload((old) => old ? { ...old, projects: next } : old)} showNotice={setNotice} />;
      case "pricing": return <PricingView rules={payload.pricingRules} />;
      case "settings": return <SettingsView settings={payload.settings} capability={payload.capability} onSave={(next) => void updateSettings(next)} onProbe={() => void probe()} saving={saving} />;
    }
  }, [loadActivity, payload, projects, saving, view]);

  return (
    <div className="app-shell">
      <aside className="sidebar">
        <div className="brand"><span className="brand-mark">CM</span><div><strong>Codex Manager</strong><small>本地桌面管理器</small></div></div>
        <nav className="side-nav" aria-label="主导航">
          {navItems.map((item) => <button type="button" key={item.id} className={view === item.id ? "is-active" : ""} onClick={() => setView(item.id)}><span>{item.label}</span><small>{item.helper}</small></button>)}
        </nav>
        <div className="sidebar-foot"><span className="runtime-mark" aria-hidden="true" />{shellLabel}<small>不做反向代理；不修改 Codex 配置。</small></div>
      </aside>
      <main className="main-area">
        <header className="topbar"><div><span className="topbar-crumb">工作台</span><span className="topbar-separator">/</span><span>{navItems.find((item) => item.id === view)?.label}</span></div><button type="button" className="button button-secondary" onClick={() => void refreshBootstrap()} disabled={loading}>{loading ? "刷新中…" : "刷新本地数据"}</button></header>
        {notice ? <InlineNotice notice={notice} onClose={() => setNotice(null)} /> : null}
        {loading && !payload ? <LoadingState /> : null}
        {error && !payload ? <div className="fatal-error"><EmptyState title="无法连接本地管理服务" detail={error} action={<button type="button" className="button button-primary" onClick={() => void refreshBootstrap()}>重试</button>} /></div> : null}
        {current}
      </main>
    </div>
  );
}
