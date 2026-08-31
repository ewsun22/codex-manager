import { useCallback, useEffect, useMemo, useRef, useState, type FormEvent, type KeyboardEvent as ReactKeyboardEvent, type ReactNode } from "react";
import { invokeBackend, isDesktopRuntime } from "./app/client.ts";
import { preferredAgentsFile } from "./app/agents.ts";
import { CodexGatewayQuickControl, CodexGatewayView } from "./app/codex-gateway.tsx";
import { CodexConfigView } from "./app/codex-config.tsx";
import { LanguageSwitcher, useI18n } from "./app/i18n.tsx";
import { t, tt } from "./app/i18n-core.ts";
import {
  MAX_UPDATE_CHECK_INTERVAL_HOURS,
  MIN_UPDATE_CHECK_INTERVAL_HOURS,
  getUpdateCheckSchedule,
  markUpdateStatusUninstallable,
  normalizeUpdateCheckIntervalHours,
} from "./app/update-scheduler.ts";
import {
  formatCount,
  formatBuildTime,
  formatDate,
  formatMetric,
  formatMs,
  formatRatio,
  formatUsd,
  metricProvenanceLabel,
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
  type AppUpdateStatus,
  type AuthProfile,
  type AuthProfileOperationResult,
  type AuthProfilesSnapshot,
  type BootstrapPayload,
  type CodexAccountSnapshot,
  type CodexLoginStartResult,
  type CodexRateLimitWindow,
  type CreateAgentsFileInput,
  type MetricValue,
  type OtelConfig,
  type PricingRule,
  type ProjectSummary,
  type SourceHealth,
  type UpdateInstallResult,
} from "./shared/contracts.ts";

type View = "overview" | "activity" | "projects" | "config" | "gateway" | "oauth" | "pricing" | "settings";

interface Notice {
  tone: "success" | "error" | "info";
  message: string;
}

function getNavItems(): Array<{ id: View; label: string; helper: string }> { return [
  { id: "overview", label: t("总览"), helper: t("采集与使用概览") },
  { id: "activity", label: t("活动记录"), helper: t("每次模型调用的元数据") },
  { id: "projects", label: t("项目与 AGENTS"), helper: t("项目发现与安全编辑") },
  { id: "config", label: t("Codex 配置"), helper: t("路由档案与安全切换") },
  { id: "gateway", label: t("本地代理"), helper: t("CLIProxyAPI 与 OAuth 凭据池") },
  { id: "oauth", label: t("官方订阅"), helper: t("账户登录、档案与额度") },
  { id: "pricing", label: t("价格目录"), helper: t("估算来源与覆盖规则") },
  { id: "settings", label: t("设置"), helper: t("本地路径与保留策略") },
]; }

function statusLabel(state: SourceHealth["state"]): string {
  return {
    healthy: t("健康"),
    degraded: t("需关注"),
    disabled: t("未启用"),
    unavailable: t("不可用"),
    unchecked: t("未检查"),
  }[state];
}

function resultLabel(result: ActivityRecord["result"]): string {
  return {
    success: t("成功"),
    failure: t("失败"),
    running: t("进行中"),
    unknown: t("未知"),
    accounted: t("已记录"),
    unobserved: t("未观测到结束"),
  }[result];
}

function uniqueStrings(values: Array<string | null>): string[] {
  return [...new Set(values.filter((value): value is string => Boolean(value)))].sort();
}

function activityQueryKey(query: ActivityQuery): string {
  return JSON.stringify(query);
}

function appError(error: unknown): string {
  return error instanceof Error ? error.message : t("操作未完成，请查看本地采集状态后重试。");
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
  showProvenance = false,
}: {
  metric: MetricValue<T>;
  format?: (value: T) => string;
  className?: string;
  showProvenance?: boolean;
}) {
  const value = formatMetric(metric, format);
  const unavailable = metric.value === null;
  return (
    <span className={`metric-value ${unavailable ? "is-unavailable" : ""} ${className}`.trim()}>
      <span>{value}</span>
      {showProvenance ? <span className="metric-source">{metricProvenanceLabel(metric)}</span> : null}
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
      <span>{t(notice.message)}</span>
      <button type="button" className="text-button" onClick={onClose}>
        {t("知道了")}
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

function LoadingState({ label = t("正在读取本地元数据…") }: { label?: string }) {
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
          <h2 id="source-health-heading">{t("来源健康")}</h2>
          <p>{t("每个指标的采集来源和缺失状态均可追溯。")}</p>
        </div>
        <span className="count-chip">{tt`${sources.length} 个来源`}</span>
      </div>
      <div className="source-list">
        {sources.map((source) => (
          <article key={source.id} className="source-item">
            <div className="source-topline">
              <strong>{source.kind === "app-server" ? t("CLI Schema 兼容性") : t(source.label)}</strong>
              <span className={`state-pill state-${source.state}`}>{statusLabel(source.state)}</span>
            </div>
            <p>{source.kind === "app-server"
              ? t(source.message ?? "仅检查 Codex CLI 是否支持 schema 命令；不读取账户、不会附着 Desktop 私有 stdio，也不影响本地采集或网关。")
              : t(source.message ?? "暂无额外说明。")}</p>
            <div className="source-meta">
              <span>{tt`最后观测：${formatDate(source.lastObservedAt)}`}</span>
              <span>{tt`滞后：${formatMs(source.lagMs)}`}</span>
              <span>{tt`未解析事件：${formatCount(source.unparsedEvents)}`}</span>
            </div>
          </article>
        ))}
      </div>
    </section>
  );
}

function Overview({ payload, onNavigate }: { payload: BootstrapPayload; onNavigate: (view: View) => void }) {
  const summary = payload.summary;
  const terminalTasks = summary ? summary.successful + summary.failed : 0;
  const successRate = summary && terminalTasks > 0 ? summary.successful / terminalTasks : null;
  const cacheRatio =
    summary?.inputTokens !== null
    && summary?.inputTokens !== undefined
    && summary.inputTokens > 0
    && summary.cachedInputTokens !== null
      ? summary.cachedInputTokens / summary.inputTokens
      : null;
  const costCoverage = summary && summary.totalCostCalls > 0
    ? tt`${formatCount(summary.pricedCalls)}/${formatCount(summary.totalCostCalls)} 次调用 · ${formatCount(summary.pricedTokens, true)}/${formatCount(summary.observedTokens, true)} token`
    : t("暂无可计价调用");

  return (
    <div className="view-content">
      <SectionHeader
        title={t("本地 Codex 运行概览")}
        subtitle={tt`${t(summary?.rangeLabel ?? "正在后台汇总已采集模型交互")} · 仅保留元数据，不读取或保存消息正文。`}
        action={
          <button type="button" className="button button-primary" onClick={() => onNavigate("activity")}>
            {t("查看活动记录")}
          </button>
        }
      />

      <section className="metric-grid" aria-label={t("核心指标")}>
        <MetricCard label={t("任务记录")} value={formatCount(summary?.records)} detail={summary ? tt`${formatCount(summary.modelCalls)} 次模型交互 · ${formatCount(summary.successful)} 成功 · ${formatCount(summary.failed)} 失败` : t("启动后在后台汇总")} />
        <MetricCard label={t("缓存命中")} value={formatRatio(cacheRatio)} detail={summary ? `${formatCount(summary.cachedInputTokens, true)} read · ${formatCount(summary.cacheWriteInputTokens, true)} write` : t("启动后在后台汇总")} tone="accent" />
        <MetricCard label={t("API 等价估算")} value={formatUsd(summary?.equivalentCostUsd, 2)} detail={summary ? tt`已覆盖 ${costCoverage}${summary.unpricedCalls ? tt` · ${formatCount(summary.unpricedCalls)} 次未覆盖` : ""}` : t("按去重后的单次 model call 计价")} tone="accent" />
        <MetricCard label={t("终态任务成功率")} value={formatRatio(successRate)} detail={summary ? t("仅已观测成功/失败终态；进行中与未观测结束不猜测") : t("启动后在后台汇总")} tone={summary?.failed ? "warn" : "default"} />
      </section>

      <section className="overview-grid">
        <section className="panel overview-activity" aria-labelledby="recent-activity-heading">
          <div className="panel-title-row">
            <div>
              <h2 id="recent-activity-heading">{t("最近活动")}</h2>
              <p>{t("任务汇总优先；token 快照已去重，模型交互数量单独统计。")}</p>
            </div>
            <button type="button" className="text-button" onClick={() => onNavigate("activity")}>
              {t("全部记录")}
            </button>
          </div>
          <div className="compact-list">
            {payload.activity.items.slice(0, 1).map((record) => (
              <button key={record.id} type="button" className="compact-row" onClick={() => onNavigate("activity")}>
                <span className={`result-dot result-${record.result}`} aria-label={resultLabel(record.result)} />
                <span className="compact-row-main">
                  <strong>{formatMetric(record.model)}</strong>
                  <small>{record.projectName ?? t("未关联项目")}</small>
                </span>
                <span className="compact-row-metric">{formatMetric(record.totalTokens, formatCount)}</span>
                <time dateTime={record.occurredAt}>{formatDate(record.occurredAt)}</time>
              </button>
            ))}
            {!payload.activity.items.length ? <EmptyState title={t("暂时没有最近活动")} detail={t("本地采集完成后，最新任务会显示在这里。")} action={<button type="button" className="button button-secondary" onClick={() => onNavigate("activity")}>{t("查看活动记录")}</button>} /> : null}
          </div>
        </section>

        <CodexGatewayQuickControl onOpenGateway={() => onNavigate("gateway")} />

        <section className="panel collection-panel" aria-labelledby="collection-heading">
          <div className="panel-title-row">
            <div>
              <h2 id="collection-heading">{t("采集边界")}</h2>
              <p>{t("默认隐私策略已生效。")}</p>
            </div>
          </div>
          <dl className="boundary-list">
            <div><dt>{t("消息正文")}</dt><dd>{t("不持久化")}</dd></div>
            <div><dt>{t("凭证与请求查询")}</dt><dd>{t("不持久化")}</dd></div>
            <div><dt>{t("路径与项目名")}</dt><dd>{t("本机 SQLite")}</dd></div>
            <div><dt>OTel receiver</dt><dd>{payload.settings.telemetryEnabled ? t("已启用") : t("未启用")}</dd></div>
          </dl>
          <button type="button" className="button button-secondary" onClick={() => onNavigate("settings")}>
            {t("查看采集设置")}
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
    <form className="filters" onSubmit={(event) => event.preventDefault()} aria-label={t("活动记录筛选")}>
      <label>
        <span>{t("搜索")}</span>
        <input
          value={query.search ?? ""}
          placeholder={t("模型、会话、项目或 endpoint")}
          onChange={(event) => onChange({ ...query, search: event.target.value || null, cursor: null })}
        />
      </label>
      <label>
        <span>{t("项目")}</span>
        <select value={query.projectPath ?? ""} onChange={(event) => onChange({ ...query, projectPath: event.target.value || null, cursor: null })}>
          <option value="">{t("全部项目")}</option>
          {projects.map((project) => <option key={project.id} value={project.canonicalPath}>{project.name}</option>)}
        </select>
      </label>
      <label>
        <span>{t("模型")}</span>
        <select value={query.model ?? ""} onChange={(event) => onChange({ ...query, model: event.target.value || null, cursor: null })}>
          <option value="">{t("全部模型")}</option>
          {models.map((model) => <option key={model} value={model}>{model}</option>)}
        </select>
      </label>
      <label>
        <span>{t("推理级别")}</span>
        <select value={query.effort ?? ""} onChange={(event) => onChange({ ...query, effort: event.target.value || null, cursor: null })}>
          <option value="">{t("全部级别")}</option>
          {efforts.map((effort) => <option key={effort} value={effort}>{effort}</option>)}
        </select>
      </label>
      <label>
        <span>{t("结果")}</span>
        <select value={query.result ?? ""} onChange={(event) => onChange({ ...query, result: (event.target.value || null) as ActivityQuery["result"], cursor: null })}>
          <option value="">{t("全部结果")}</option>
          <option value="success">{t("成功")}</option>
          <option value="failure">{t("失败")}</option>
          <option value="running">{t("进行中")}</option>
          <option value="unobserved">{t("未观测到结束")}</option>
          {query.view === "modelCalls" ? <option value="accounted">{t("已记录的模型交互")}</option> : null}
          <option value="unknown">{t("未知")}</option>
        </select>
      </label>
      <button type="button" className="text-button filter-reset" onClick={onReset}>{t("清除筛选")}</button>
    </form>
  );
}

function timingUnavailableReason(record: ActivityRecord, field: "firstVisibleOutput" | "duration"): string {
  if (record.activityKind === "modelCall") return t("该条是 token 结算记录；本地 rollout 未提供调用级耗时。");
  if (record.activityKind === "otelRequest" && field === "firstVisibleOutput") return t("OTel 请求记录未提供首个输出时间。");
  if (record.activityKind === "otelRequest") return t("OTel 请求记录未提供总耗时。");
  if (record.result === "running") return t("任务仍在进行，尚未产生完整耗时。");
  if (record.result === "unobserved") return t("未观测到任务结束事件，无法计算完整耗时。");
  return t("当前采集来源未提供该时间指标。");
}

function TimingValue({ record, metric, field }: { record: ActivityRecord; metric: MetricValue<number>; field: "firstVisibleOutput" | "duration" }) {
  if (record.timingScope === "unavailable" || metric.value === null) {
    return <span className="timing-missing" title={timingUnavailableReason(record, field)}><span>—</span><small>{timingUnavailableReason(record, field)}</small></span>;
  }
  return <ValueWithProvenance metric={metric} format={formatMs} />;
}

function ResultPill({ result, statusCode }: { result: ActivityRecord["result"]; statusCode?: number | null }) {
  return <span className={`result-pill result-${result}`}>{resultLabel(result)}{statusCode ? ` · ${statusCode}` : ""}</span>;
}

function ActivityTable({ page, view, onSelect }: { page: ActivityPage; view: NonNullable<ActivityQuery["view"]>; onSelect: (record: ActivityRecord) => void }) {
  if (!page.items.length) {
    return <EmptyState title={view === "turns" ? t("没有匹配的任务汇总") : t("没有匹配的模型交互")} detail={t("调整筛选条件，或等待下一轮本地采集完成。")} />;
  }
  const turns = view === "turns";
  return (
    <div className="table-scroll">
      <table className="activity-table">
        <thead>
          <tr>
            <th scope="col">{t("时间")}</th>
            <th scope="col">{t("模型 / 推理")}</th>
            <th scope="col">{t("项目")}</th>
            <th scope="col">{t("输入")}</th>
            <th scope="col">{t("缓存读")}</th>
            <th scope="col">{t("缓存写")}</th>
            <th scope="col">{t("输出")}</th>
            <th scope="col">{t("总计")}</th>
            <th scope="col">{turns ? t("模型调用") : t("所属任务")}</th>
            {!turns ? <th scope="col">{t("记录状态")}</th> : null}
            <th scope="col">{t("首个输出")}</th>
            <th scope="col">{t("总耗时")}</th>
            {turns ? <th scope="col">{t("任务状态")}</th> : null}
          </tr>
        </thead>
        <tbody>
          {page.items.map((record) => (
            <tr key={record.id} tabIndex={0} role="button" aria-label={tt`查看 ${record.projectName ?? t("未关联项目")} 于 ${formatDate(record.occurredAt)} 的详情`} onClick={() => onSelect(record)} onKeyDown={(event) => {
              if (event.key === "Enter" || event.key === " ") {
                event.preventDefault();
                onSelect(record);
              }
            }}>
              <td><time dateTime={record.occurredAt}>{formatDate(record.occurredAt)}</time></td>
              <td><strong>{formatMetric(record.model)}</strong><small>{record.activityKind === "otelRequest" ? t("OTel 请求") : formatMetric(record.effort)}</small></td>
              <td><span className="project-cell" title={record.projectPath ?? record.projectName ?? t("未关联项目")}>{record.projectName ?? t("未关联项目")}</span></td>
              <td><ValueWithProvenance metric={record.inputTokens} format={formatCount} /></td>
              <td><ValueWithProvenance metric={record.cachedInputTokens} format={formatCount} /></td>
              <td><ValueWithProvenance metric={record.cacheWriteInputTokens} format={formatCount} /></td>
              <td><ValueWithProvenance metric={record.outputTokens} format={formatCount} /></td>
              <td><ValueWithProvenance metric={record.totalTokens} format={formatCount} className="strong-value" /></td>
              {turns ? <td><strong>{formatCount(record.modelCallCount)}</strong><small>{t("次已去重调用")}</small></td> : <td>{record.parentTurnResult ? <ResultPill result={record.parentTurnResult} /> : <span className="muted">{t("未关联任务")}</span>}</td>}
              {!turns ? <td><ResultPill result={record.result} /></td> : null}
              <td><TimingValue record={record} metric={record.firstVisibleOutputMs} field="firstVisibleOutput" /></td>
              <td><TimingValue record={record} metric={record.durationMs} field="duration" /></td>
              {turns ? <td><ResultPill result={record.result} statusCode={record.statusCode} /></td> : null}
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

function ActivityDrawer({ record, onClose }: { record: ActivityRecord; onClose: () => void }) {
  const items: Array<[string, MetricValue<string | number>, (value: string | number) => string]> = [
    [t("模型"), record.model, String],
    [t("推理级别"), record.effort, String],
    ["Provider", record.provider, String],
    [t("请求地址"), record.endpoint, String],
    [t("输入 token"), record.inputTokens, (value) => formatCount(Number(value))],
    [t("缓存输入"), record.cachedInputTokens, (value) => formatCount(Number(value))],
    [t("缓存写入"), record.cacheWriteInputTokens, (value) => formatCount(Number(value))],
    [t("输出 token"), record.outputTokens, (value) => formatCount(Number(value))],
    [t("推理输出"), record.reasoningOutputTokens, (value) => formatCount(Number(value))],
    [t("总 token"), record.totalTokens, (value) => formatCount(Number(value))],
    [t("缓存命中率"), record.cacheHitRatio, (value) => formatRatio(Number(value))],
    [t("API 等价估算"), record.equivalentCostUsd, (value) => formatUsd(Number(value))],
    [t("首个输出"), record.firstVisibleOutputMs, (value) => formatMs(Number(value))],
    [t("全程耗时"), record.durationMs, (value) => formatMs(Number(value))],
    [t("响应字节"), record.responseBytes, (value) => formatCount(Number(value))],
  ];
  return (
    <div className="drawer-backdrop" role="presentation" onMouseDown={onClose}>
      <aside className="drawer" role="dialog" aria-modal="true" aria-labelledby="activity-detail-title" onMouseDown={(event) => event.stopPropagation()}>
        <div className="drawer-header">
          <div>
            <p className="eyebrow">{record.activityKind === "turn" ? t("任务汇总详情") : record.activityKind === "otelRequest" ? t("OTel 请求详情") : t("模型交互详情")}</p>
            <h2 id="activity-detail-title">{formatMetric(record.model)}</h2>
            <p>{formatDate(record.occurredAt)} · {record.projectName ?? t("未关联项目")}</p>
          </div>
          <button type="button" className="button button-secondary" onClick={onClose}>{t("关闭")}</button>
        </div>
        <div className="detail-callout">
          <ResultPill result={record.result} statusCode={record.statusCode} />
          {record.activityKind !== "turn" ? <span>{tt`所属任务：${record.parentTurnResult ? resultLabel(record.parentTurnResult) : t("未关联")}`}</span> : <span>{tt`模型调用：${formatCount(record.modelCallCount)} 次`}</span>}
          <span>{tt`会话 ${record.sessionId}`}</span>
          {record.activityKind === "modelCall" ? <span>{tt`耗时：${timingUnavailableReason(record, "duration")}`}</span> : null}
        </div>
        <dl className="detail-list">
          {items.map(([label, metric, format]) => (
            <div key={label}>
              <dt>{label}</dt>
              <dd><ValueWithProvenance metric={metric} format={format} showProvenance /></dd>
            </div>
          ))}
        </dl>
        <p className="drawer-footnote">{t("本页仅显示归一化后的元数据；请求正文、Authorization 与 Cookie 不会进入界面或数据库。")}</p>
      </aside>
    </div>
  );
}

function PaginationControls({
  pageNumber,
  pageSize,
  hasPrevious,
  hasNext,
  disabled,
  onPrevious,
  onNext,
  compact = false,
}: {
  pageNumber: number;
  pageSize: number;
  hasPrevious: boolean;
  hasNext: boolean;
  disabled: boolean;
  onPrevious: () => void;
  onNext: () => void;
  compact?: boolean;
}) {
  return (
    <nav className={`pagination ${compact ? "pagination-compact" : ""}`} aria-label={t("活动记录分页")}>
      <button type="button" className="button button-secondary" onClick={onPrevious} disabled={disabled || !hasPrevious}>{t("上一页")}</button>
      <span aria-live="polite">{tt`第 ${pageNumber} 页 · 每页 ${pageSize} 条`}</span>
      <button type="button" className="button button-secondary" onClick={onNext} disabled={disabled || !hasNext}>{t("下一页")}</button>
    </nav>
  );
}

function ActivityView({
  initial,
  projects,
  onLoad,
  refreshRevision,
  fullRefreshRevision,
}: {
  initial: ActivityPage;
  projects: ProjectSummary[];
  onLoad: (query: ActivityQuery) => Promise<ActivityPage>;
  refreshRevision: number;
  fullRefreshRevision: number;
}) {
  const [query, setQuery] = useState<ActivityQuery>({ limit: 50, view: "turns" });
  const [page, setPage] = useState<ActivityPage>(initial);
  const [selected, setSelected] = useState<ActivityRecord | null>(null);
  const [loading, setLoading] = useState(false);
  const [syncing, setSyncing] = useState(false);
  const [updatedAt, setUpdatedAt] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [cursorHistory, setCursorHistory] = useState<Array<string | null>>([]);
  const queryRef = useRef(query);
  const mountedRef = useRef(true);
  const loadInFlight = useRef(false);
  const activeLoad = useRef<{ id: number; query: ActivityQuery; blocking: boolean; countTotal: boolean } | null>(null);
  const revisionProbeInFlight = useRef(false);
  const revisionRef = useRef(initial.revision);
  const latestLoadId = useRef(0);
  const pendingLoad = useRef<{ id: number; query: ActivityQuery; blocking: boolean; countTotal: boolean } | null>(null);
  const observedQueryKey = useRef(activityQueryKey(query));
  const turnTabRef = useRef<HTMLButtonElement>(null);
  const modelCallsTabRef = useRef<HTMLButtonElement>(null);

  queryRef.current = query;

  const requestLoad = useCallback((nextQuery: ActivityQuery, blocking: boolean, countTotal = blocking) => {
    const request = { id: ++latestLoadId.current, query: nextQuery, blocking, countTotal };
    const run = (current: typeof request) => {
      loadInFlight.current = true;
      activeLoad.current = current;
      if (current.blocking) setLoading(true);
      setSyncing(true);
      setError(null);
      void onLoad({ ...current.query, refreshOnly: !current.countTotal })
        .then((next) => {
          if (mountedRef.current && current.id === latestLoadId.current && activityQueryKey(queryRef.current) === activityQueryKey(current.query)) {
            revisionRef.current = next.revision;
            setPage((previous) => ({
              ...next,
              total: next.total ?? previous.total,
            }));
            setUpdatedAt(new Date().toISOString());
          }
        })
        .catch((nextError: unknown) => {
          if (mountedRef.current && current.id === latestLoadId.current) setError(appError(nextError));
        })
        .finally(() => {
          if (!mountedRef.current) {
            pendingLoad.current = null;
            loadInFlight.current = false;
            return;
          }
          const pending = pendingLoad.current;
          pendingLoad.current = null;
          loadInFlight.current = false;
          activeLoad.current = null;
          if (current.blocking) setLoading(false);
          if (pending) {
            run(pending);
          } else {
            setSyncing(false);
          }
        });
    };

    if (loadInFlight.current) {
      const pending = pendingLoad.current;
      const active = activeLoad.current;
      const matchesActiveQuery = active !== null
        && activityQueryKey(active.query) === activityQueryKey(request.query);
      pendingLoad.current = {
        ...request,
        // A user-initiated query or page change must retain its full loading state
        // when it overtakes a background refresh.
        blocking: request.blocking || pending?.blocking === true
          || (matchesActiveQuery && active.blocking),
        countTotal: request.countTotal || pending?.countTotal === true
          || (matchesActiveQuery && active.countTotal),
      };
      return;
    }
    run(request);
  }, [onLoad]);

  const pollRevision = useCallback(() => {
    if (revisionProbeInFlight.current) return;
    revisionProbeInFlight.current = true;
    void onLoad({ revisionOnly: true })
      .then((probe) => {
        if (mountedRef.current && probe.revision !== revisionRef.current) {
          requestLoad(queryRef.current, false, false);
        }
      })
      .catch(() => undefined)
      .finally(() => {
        revisionProbeInFlight.current = false;
      });
  }, [onLoad, requestLoad]);

  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
      latestLoadId.current += 1;
      pendingLoad.current = null;
      activeLoad.current = null;
    };
  }, []);

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
  const handleViewTabKeyDown = (event: ReactKeyboardEvent<HTMLButtonElement>) => {
    const currentView = query.view ?? "turns";
    const nextView = event.key === "Home"
      ? "turns"
      : event.key === "End"
        ? "modelCalls"
        : event.key === "ArrowRight" || event.key === "ArrowLeft"
          ? currentView === "turns" ? "modelCalls" : "turns"
          : null;
    if (!nextView) return;
    event.preventDefault();
    updateQuery({ ...query, view: nextView });
    window.requestAnimationFrame(() => {
      (nextView === "turns" ? turnTabRef : modelCallsTabRef).current?.focus();
    });
  };

  useEffect(() => {
    // Bootstrap already contains the first activity page. Do not immediately
    // repeat the same expensive query when the user opens this view.
    const nextQueryKey = activityQueryKey(query);
    if (nextQueryKey === observedQueryKey.current) return;
    observedQueryKey.current = nextQueryKey;
    requestLoad(query, true, true);
  }, [query, requestLoad]);

  useEffect(() => {
    if (page.total === null && initial.total !== null) {
      setPage((current) => ({ ...current, total: initial.total }));
    }
  }, [initial.total, page.total]);

  useEffect(() => {
    if (refreshRevision > 0) requestLoad(queryRef.current, false, false);
  }, [refreshRevision, requestLoad]);

  useEffect(() => {
    if (fullRefreshRevision > 0) requestLoad(queryRef.current, false, true);
  }, [fullRefreshRevision, requestLoad]);

  useEffect(() => {
    const timer = window.setInterval(pollRevision, 5_000);
    return () => window.clearInterval(timer);
  }, [pollRevision]);

  return (
    <div className="view-content">
      <SectionHeader title={t("活动记录")} subtitle={t("默认按任务汇总展示真实状态和任务级耗时；模型交互保留 token 结算与 OTel 请求，不把任务状态或耗时伪造为调用结果。")} />
      <div className="activity-view-tabs" role="tablist" aria-label={t("活动记录视图")}>
        <button ref={turnTabRef} type="button" role="tab" aria-selected={(query.view ?? "turns") === "turns"} tabIndex={(query.view ?? "turns") === "turns" ? 0 : -1} className={(query.view ?? "turns") === "turns" ? "is-active" : ""} onClick={() => updateQuery({ ...query, view: "turns" })} onKeyDown={handleViewTabKeyDown}>{t("任务汇总")}</button>
        <button ref={modelCallsTabRef} type="button" role="tab" aria-selected={query.view === "modelCalls"} tabIndex={query.view === "modelCalls" ? 0 : -1} className={query.view === "modelCalls" ? "is-active" : ""} onClick={() => updateQuery({ ...query, view: "modelCalls" })} onKeyDown={handleViewTabKeyDown}>{t("模型交互")}</button>
      </div>
      <ActivityFilters
        query={query}
        records={page.items}
        projects={projects}
        onChange={updateQuery}
        onReset={() => { setCursorHistory([]); setQuery({ limit: 50, view: query.view ?? "turns" }); }}
      />
      <section className="panel table-panel" aria-labelledby="activity-table-heading">
        <div className="panel-title-row">
          <div>
            <h2 id="activity-table-heading">{formatCount(page.total)} {(query.view ?? "turns") === "turns" ? t("条任务") : t("条模型交互")}</h2>
            <p><span role="status" aria-live="polite">{t("每 5 秒自动更新")}{syncing ? t(" · 正在同步…") : updatedAt ? tt` · 最近更新 ${formatDate(updatedAt)}` : ""}</span></p>
            <details className="data-explanation">
              <summary>{t("数据说明")}</summary>
              <p>{t("表格默认只显示数值。打开详情可查看每个字段来自 Codex 本地记录、推导计算、价格目录或服务端/OTel 元数据；这些来源说明不等同于账单或服务端审计日志。")}</p>
            </details>
          </div>
          <div className="activity-table-controls">
            <label className="page-size">
              <span>{t("每页")}</span>
              <select aria-label={t("每页显示数量")} value={query.limit ?? 50} onChange={(event) => updateQuery({ ...query, limit: Number(event.target.value) })}>
                <option value={25}>25</option>
                <option value={50}>50</option>
                <option value={100}>100</option>
              </select>
            </label>
            <PaginationControls pageNumber={cursorHistory.length + 1} pageSize={query.limit ?? 50} hasPrevious={cursorHistory.length > 0} hasNext={Boolean(page.nextCursor)} disabled={loading} onPrevious={goPrevious} onNext={goNext} compact />
          </div>
        </div>
        {error ? <InlineNotice notice={{ tone: "error", message: error }} onClose={() => setError(null)} /> : null}
        {loading ? <LoadingState label={t("正在应用筛选…")} /> : <ActivityTable page={page} view={query.view ?? "turns"} onSelect={setSelected} />}
        <PaginationControls pageNumber={cursorHistory.length + 1} pageSize={query.limit ?? 50} hasPrevious={cursorHistory.length > 0} hasNext={Boolean(page.nextCursor)} disabled={loading} onPrevious={goPrevious} onNext={goNext} />
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
          <h2 id="projects-heading">{t("已发现项目")}</h2>
          <p>{t("来自本地观测路径和你授权的根目录。")}</p>
          <div className="agents-status-legend" aria-label={t("AGENTS 文件状态图例")}>
            <span><i className="agents-file-indicator has-agents-file" aria-hidden="true" />{t("有 AGENTS.md")}</span>
            <span><i className="agents-file-indicator no-agents-file" aria-hidden="true" />{t("无 AGENTS.md")}</span>
          </div>
        </div>
        <button type="button" className="button button-secondary" onClick={onDiscover} disabled={discovering}>
          {discovering ? t("扫描中…") : t("重新发现")}
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
                <span className="project-row-heading">
                  <strong>{project.name}</strong>
                  <time dateTime={project.lastConversationAt ?? undefined}>{project.lastConversationAt ? tt`最后对话 ${formatDate(project.lastConversationAt)}` : t("尚未观测到对话")}</time>
                </span>
                <small>{project.canonicalPath}</small>
              </span>
              <span className="project-flags">
                <span className="agents-file-status" role="img" aria-label={project.hasAgentsFile ? t("项目根目录存在 AGENTS.md") : t("项目根目录没有 AGENTS.md")} title={project.hasAgentsFile ? t("项目根目录存在 AGENTS.md") : t("项目根目录没有 AGENTS.md")}>
                  <i className={`agents-file-indicator ${project.hasAgentsFile ? "has-agents-file" : "no-agents-file"}`} aria-hidden="true" />
                </span>
                {project.isGit ? <span>Git</span> : null}
                {project.worktree ? <span>Worktree</span> : null}
                <span>{tt`${project.agentsFileCount} 个 AGENTS`}</span>
              </span>
            </button>
          ))}
        </div>
      ) : <EmptyState title={t("尚未发现项目")} detail={t("先在设置中授权一个项目根目录，再执行发现。")} />}
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
    return <section className="panel agents-editor-panel"><EmptyState title={t("请选择一个项目")} detail={t("选择项目后会解析当前工作目录的 AGENTS 层级。")} /></section>;
  }

  const changed = Boolean(snapshot && baseHash === snapshot.sha256 && content !== snapshot.content);
  return (
    <section className="panel agents-editor-panel" aria-labelledby="agents-editor-heading">
      <div className="panel-title-row">
        <div>
          <h2 id="agents-editor-heading">{t("AGENTS 层级与编辑器")}</h2>
          <p>{t("有效文件由后端按 canonical path 与授权根目录确认；应用不会执行文件内容。")}</p>
        </div>
        <div className="agents-header-actions">
          {snapshot ? <span className={`state-pill ${snapshot.writable && canSave ? "state-healthy" : "state-unavailable"}`}>{snapshot.writable && canSave ? t("可编辑") : t("未授权或只读")}</span> : null}
          {canCreate ? <button type="button" className="button button-secondary" onClick={onCreate} disabled={busy}>{busy ? t("创建中…") : t("创建项目级 AGENTS.md")}</button> : null}
        </div>
      </div>

      <div className="chain-summary">
        <span>{t("当前目录：")}<code>{chain.selectedCwd}</code></span>
        <span>{tt`有效规则：${chain.effectivePaths.length ? chain.effectivePaths.length : "unavailable"}`}</span>
        <span>{tt`单文件上限：${formatCount(chain.maxBytes)} B`}</span>
      </div>

      <div className="agents-layout">
        <div className="agents-file-list" role="list" aria-label={t("AGENTS 文件链")}>
          {chain.files.map((file) => (
            <button
              type="button"
              role="listitem"
              key={file.path}
              className={`agents-file-row ${snapshot?.path === file.path ? "is-open" : ""}`}
              onClick={() => onOpen(file.path)}
            >
              <strong>{file.relativePath}</strong>
              <small>{tt`${file.kind} · 优先级 ${file.precedence}`}</small>
              <span>{file.effective ? t("当前有效") : file.overridden ? t("已被覆盖") : t("候选")}</span>
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
              <label className="sr-only" htmlFor="agents-content">{t("AGENTS.md 内容")}</label>
              <textarea
                id="agents-content"
                className="agents-textarea"
                value={content}
                onChange={(event) => setContent(event.target.value)}
                spellCheck={false}
                readOnly={!snapshot.writable}
              />
              <div className="editor-actions">
                <span className={changed ? "changed-indicator" : "muted"}>{changed ? t("存在未保存修改") : t("已与磁盘快照一致")}</span>
                <button type="button" className="button button-secondary" onClick={() => setContent(snapshot.content)} disabled={!changed || busy}>{t("放弃修改")}</button>
                <button type="button" className="button button-primary" onClick={() => onSave(content, snapshot)} disabled={!changed || !snapshot.writable || !canSave || busy}>{busy ? t("保存中…") : t("安全保存")}</button>
              </div>
            </>
          ) : (
            <EmptyState
              title={t("此项目没有项目级 AGENTS.md")}
              detail={canCreate ? t("可在已授权的项目根目录中显式创建 AGENTS.md；创建后才会进入安全编辑和 revision 流程。") : t("文件不存在、位于未授权根目录、或未通过符号链接检查时，均不会显示为空白编辑器。")}
              action={canCreate ? <button type="button" className="button button-primary" onClick={onCreate} disabled={busy}>{busy ? t("创建中…") : t("创建项目级 AGENTS.md")}</button> : undefined}
            />
          )}
        </div>
      </div>

      {chain.warnings.length ? <div className="warning-list" role="status">{chain.warnings.map((warning) => <p key={warning}>{t(warning)}</p>)}</div> : null}

      <div className="revision-section">
        <div className="panel-title-row">
          <div><h3>{t("本地 revision")}</h3><p>{t("仅列出由本应用安全保存时生成的版本。")}</p></div>
        </div>
        {revisions.length ? (
          <div className="revision-list">
            {revisions.map((revision) => (
              <div key={revision.id} className="revision-row">
                <span><strong>{formatDate(revision.createdAt)}</strong><small>{revision.afterSha256} · {formatCount(revision.byteLength)} B</small></span>
                <button type="button" className="text-button" onClick={() => onRestore(revision)} disabled={busy}>{t("恢复此版本")}</button>
              </div>
            ))}
          </div>
        ) : <p className="muted">{t("暂无由本应用创建的 revision。")}</p>}
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
      const candidate = preferredAgentsFile(nextChain);
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
      showNotice({ tone: "error", message: tt`无法读取 AGENTS 层级：${appError(error)}` });
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
      showNotice({ tone: "error", message: tt`无法打开文件：${appError(error)}` });
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
        showNotice({ tone: "error", message: t("该文件不位于已授权根目录内，应用不会发送写入请求。") });
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
      showNotice({ tone: "success", message: tt`已原子保存，并创建 revision ${result.revisionId}。` });
    } catch (error: unknown) {
      showNotice({ tone: "error", message: tt`保存被拒绝或发生外部冲突：${appError(error)}` });
    } finally {
      setLoading(false);
    }
  };

  const createProjectAgents = async () => {
    if (!selectedProject) return;
    const authorizedRoot = authorizedRootFor(selectedProject.canonicalPath, authorizedRoots);
    if (!authorizedRoot) {
      showNotice({ tone: "error", message: t("项目根目录不在授权范围内，应用不会创建文件。") });
      return;
    }
    const confirmed = window.confirm(tt`将在项目根目录创建 AGENTS.md：\n${selectedProject.canonicalPath}\n\n继续吗？`);
    if (!confirmed) return;
    setLoading(true);
    try {
      const input: CreateAgentsFileInput = {
        projectPath: selectedProject.canonicalPath,
        authorizedRoot,
        content: t("# 项目 AGENTS 说明\n\n## 工作边界\n\n- 在此处维护该项目的 Codex 协作规则。\n- 修改后请运行相关测试并报告实际结果。\n"),
        fileName: "AGENTS.md",
      };
      const result = await invokeBackend<{ snapshot: AgentsFileSnapshot; revisionId: string }>(COMMANDS.createAgentsFile, { input });
      setSnapshot(result.snapshot);
      setRevisions(await invokeBackend<AgentsRevision[]>(COMMANDS.listAgentsRevisions, { path: result.snapshot.path }));
      const [refreshedChain, refreshedProjects] = await Promise.all([
        invokeBackend<AgentsChain>(COMMANDS.getAgentsChain, {
          projectPath: selectedProject.canonicalPath,
          selectedCwd: selectedProject.canonicalPath,
        }),
        invokeBackend<ProjectSummary[]>(COMMANDS.listProjects),
      ]);
      setChain(refreshedChain);
      onProjectsChange(refreshedProjects);
      showNotice({ tone: "success", message: tt`已创建项目级 AGENTS.md，并生成 revision ${result.revisionId}。` });
    } catch (error: unknown) {
      showNotice({ tone: "error", message: tt`创建被拒绝或未完成：${appError(error)}` });
    } finally {
      setLoading(false);
    }
  };

  const restore = async (revision: AgentsRevision) => {
    if (!snapshot) return;
    const confirmed = window.confirm(tt`恢复 ${formatDate(revision.createdAt)} 的 revision？当前磁盘内容会先由后端进行冲突校验。`);
    if (!confirmed) return;
    setLoading(true);
    try {
      const result = await invokeBackend<{ snapshot: AgentsFileSnapshot; revisionId: string }>(COMMANDS.restoreAgentsRevision, {
        revisionId: revision.id,
        expectedSha256: snapshot.sha256,
      });
      setSnapshot(result.snapshot);
      setRevisions(await invokeBackend<AgentsRevision[]>(COMMANDS.listAgentsRevisions, { path: result.snapshot.path }));
      showNotice({ tone: "success", message: tt`已恢复 revision，并创建 ${result.revisionId}。` });
    } catch (error: unknown) {
      showNotice({ tone: "error", message: tt`恢复被拒绝或发生外部冲突：${appError(error)}` });
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
      showNotice({ tone: "success", message: tt`项目发现完成：${formatCount(next.length)} 个项目。` });
    } catch (error: unknown) {
      showNotice({ tone: "error", message: tt`项目发现未完成：${appError(error)}` });
    } finally {
      setDiscovering(false);
    }
  };

  return (
    <div className="view-content">
      <SectionHeader title={t("项目与 AGENTS")} subtitle={t("查看所有已观测项目，并在授权根目录内安全编辑 AGENTS.md。")} />
      <div className="projects-grid">
        <ProjectList projects={projects} selectedPath={selectedProject?.canonicalPath ?? null} onSelect={setSelectedProject} onDiscover={discover} discovering={discovering} />
        {loading && !chain ? <LoadingState label={t("正在解析项目层级…")} /> : <AgentsEditor chain={chain} snapshot={snapshot} revisions={revisions} busy={loading} canSave={Boolean(snapshot && authorizedRootFor(snapshot.path, authorizedRoots))} canCreate={canCreate} onOpen={(path) => void openFile(path)} onSave={(content, current) => void save(content, current)} onRestore={(revision) => void restore(revision)} onCreate={() => void createProjectAgents()} />}
      </div>
    </div>
  );
}

function PricingView({ rules }: { rules: PricingRule[] }) {
  return (
    <div className="view-content">
      <SectionHeader title={t("价格目录")} subtitle={t("价格只用于 API 等价估算；ChatGPT 套餐与第三方中转站真实账单均显示 unavailable。")} />
      <section className="panel table-panel" aria-labelledby="pricing-heading">
        <div className="panel-title-row">
          <div><h2 id="pricing-heading">{t("生效规则")}</h2><p>{t("每条 API 等价估算都保留 catalog 生效日期与来源。")}</p></div>
          <span className="count-chip">{tt`${rules.length} 条规则`}</span>
        </div>
        {rules.length ? (
          <div className="table-scroll"><table className="pricing-table"><thead><tr><th>Provider</th><th>{t("模型匹配")}</th><th>{t("输入 / M")}</th><th>{t("缓存读 / M")}</th><th>{t("缓存写 / M")}</th><th>{t("输出 / M")}</th><th>{t("生效日期")}</th><th>{t("来源")}</th></tr></thead><tbody>
            {rules.map((rule) => <tr key={rule.id}><td>{rule.provider}</td><td><code>{rule.modelPattern}</code></td><td>{formatUsd(rule.inputPerMillion)}</td><td>{formatUsd(rule.cachedInputPerMillion)}</td><td>{formatUsd(rule.cacheWriteInputPerMillion)}</td><td>{formatUsd(rule.outputPerMillion)}</td><td>{rule.effectiveFrom}</td><td>{rule.isUserOverride ? t("用户覆盖") : rule.sourceLabel}</td></tr>)}
          </tbody></table></div>
        ) : <EmptyState title={t("价格目录不可用")} detail={t("未匹配到规则时，调用详情会显示 unavailable 而不是显示零成本。")} />}
      </section>
    </div>
  );
}

type OAuthTab = "login" | "credentials" | "quota";

const oauthTabs: Array<{ id: OAuthTab; label: string }> = [
  { id: "login", label: t("官方登录") },
  { id: "credentials", label: t("订阅账户") },
  { id: "quota", label: t("额度与重置") },
];

function authMethodLabel(method: string | null): string {
  return {
    chatgpt: "ChatGPT OAuth",
    apiKey: "OpenAI API Key",
    amazonBedrock: "Amazon Bedrock",
  }[method ?? ""] ?? (method || "unavailable");
}

function accountStateLabel(snapshot: CodexAccountSnapshot): string {
  if (snapshot.loginInProgress) return t("登录中");
  return {
    authenticated: t("已登录"),
    "signed-out": t("未登录"),
    unavailable: "unavailable",
  }[snapshot.state];
}

function accountCacheLabel(snapshot: CodexAccountSnapshot): string {
  if (snapshot.stale || snapshot.source === "stale") return t("上次确认数据（可能已过期）");
  if (snapshot.source === "cache") return t("本地缓存");
  if (snapshot.source === "live") return t("刚刚确认");
  return t("状态来源 unavailable");
}

function accountCachedAt(snapshot: CodexAccountSnapshot): string | null {
  return snapshot.cachedAt;
}

function quotaWindowLabel(window: CodexRateLimitWindow): string {
  if (window.windowDurationMins === null) return t("额度窗口");
  if (window.windowDurationMins % 10_080 === 0) return tt`${window.windowDurationMins / 10_080} 周窗口`;
  if (window.windowDurationMins % 1_440 === 0) return tt`${window.windowDurationMins / 1_440} 天窗口`;
  if (window.windowDurationMins % 60 === 0) return tt`${window.windowDurationMins / 60} 小时窗口`;
  return tt`${window.windowDurationMins} 分钟窗口`;
}

function QuotaWindow({ window, label }: { window: CodexRateLimitWindow; label: string }) {
  const remaining = Math.max(0, Math.min(100 - window.usedPercent, 100));
  return (
    <div className="quota-window">
      <div className="quota-window-heading">
        <span>{label} · {quotaWindowLabel(window)}</span>
        <strong>{tt`剩余 ${remaining.toFixed(0)}%`}</strong>
      </div>
      <div className="quota-track" role="progressbar" aria-label={tt`${label}剩余额度`} aria-valuemin={0} aria-valuemax={100} aria-valuenow={Math.round(remaining)}>
        <span style={{ width: `${remaining}%` }} />
      </div>
      <small>{tt`重置时间：${formatDate(window.resetsAt)}`}</small>
    </div>
  );
}

function AuthProfilesPanel({
  snapshot,
  loading,
  busy,
  label,
  onLabelChange,
  onRefresh,
  onImport,
  onActivate,
  onDelete,
  onRestore,
}: {
  snapshot: AuthProfilesSnapshot | null;
  loading: boolean;
  busy: string | null;
  label: string;
  onLabelChange: (label: string) => void;
  onRefresh: () => void;
  onImport: () => void;
  onActivate: (profile: AuthProfile) => void;
  onDelete: (profile: AuthProfile) => void;
  onRestore: (profile: AuthProfile) => void;
}) {
  const available = snapshot?.profiles.filter((profile) => profile.deletedAt === null) ?? [];
  const deleted = snapshot?.profiles.filter((profile) => profile.deletedAt !== null) ?? [];
  const activeIndex = available.findIndex((profile) => profile.isActive);
  const nextProfile = available.length > 1
    ? available[(activeIndex >= 0 ? activeIndex + 1 : 0) % available.length]
    : null;

  return (
    <div className="auth-profile-stack">
      <section className="panel auth-profile-toolbar" aria-labelledby="auth-profile-heading">
        <div className="panel-title-row">
          <div><h2 id="auth-profile-heading">{t("认证档案")}</h2><p>{t("导入文件由 Rust 原生选择器读取；前端不会获得路径、JSON 或 token。")}</p></div>
          <span className="count-chip">{tt`${available.length} 个可用`}</span>
        </div>
        <div className="auth-profile-import-row">
          <label><span>{t("档案名称")}</span><input value={label} maxLength={64} onChange={(event) => onLabelChange(event.target.value)} placeholder={t("例如：个人 Pro")} disabled={busy !== null} /></label>
          <button type="button" className="button button-primary" onClick={onImport} disabled={busy !== null || !label.trim()}>{busy === "import" ? t("验证并导入中…") : t("选择 JSON 并导入")}</button>
          <button type="button" className="button button-secondary" onClick={onRefresh} disabled={busy !== null || loading}>{loading ? t("读取中…") : t("刷新档案")}</button>
          <button type="button" className="button button-secondary" onClick={() => nextProfile && onActivate(nextProfile)} disabled={busy !== null || !nextProfile || !snapshot?.activationAvailable || !snapshot.activeRevision}>{t("轮换到下一个")}</button>
        </div>
        <p className="capability-message">{snapshot?.message ? t(snapshot.message) : t("正在读取认证档案…")}</p>
      </section>

      {loading && !snapshot ? <LoadingState label={t("正在读取 macOS Keychain 认证档案…")} /> : null}
      {snapshot && !snapshot.supported ? <EmptyState title={t("当前平台不支持认证档案")} detail={t("首版只支持 macOS Keychain 与 file 模式 auth.json。")} /> : null}
      {snapshot && available.length === 0 ? <EmptyState title={t("还没有认证档案")} detail={t("输入不含邮箱或路径的本地名称，然后从原生选择器导入官方 Codex auth.json。")} /> : null}

      {available.map((profile) => (
        <section className={`panel auth-profile-row ${profile.isActive ? "is-active" : ""}`} key={profile.id}>
          <div className="auth-profile-main">
            <span className="oauth-provider-mark auth-profile-mark" aria-hidden="true">›_</span>
            <div><h3>{profile.label}</h3><p>{tt`更新：${formatDate(profile.updatedAt)} · 凭据保存在应用专用 Keychain`}</p></div>
            <span className={`state-pill ${profile.isActive ? "state-healthy" : "state-disabled"}`}>{profile.isActive ? t("当前使用") : t("待切换")}</span>
          </div>
          <div className="auth-profile-actions">
            <button type="button" className="button button-secondary" onClick={() => onActivate(profile)} disabled={busy !== null || profile.isActive || !snapshot?.activationAvailable || !snapshot.activeRevision}>{busy === `activate:${profile.id}` ? t("切换中…") : t("设为当前")}</button>
            <button type="button" className="button button-danger" onClick={() => onDelete(profile)} disabled={busy !== null || profile.isActive || available.length <= 1}>{busy === `delete:${profile.id}` ? t("删除中…") : t("删除")}</button>
          </div>
        </section>
      ))}

      {deleted.length > 0 ? (
        <section className="panel auth-profile-trash">
          <div className="panel-title-row"><div><h2>{t("回收站")}</h2><p>{tt`软删除档案保留 ${snapshot?.deletedRetentionDays ?? 30} 天，之后从应用专用 Keychain 清理。`}</p></div><span className="count-chip">{deleted.length}</span></div>
          {deleted.map((profile) => (
            <div className="auth-profile-trash-row" key={profile.id}>
              <div><strong>{profile.label}</strong><small>{tt`删除：${formatDate(profile.deletedAt)}`}</small></div>
              <button type="button" className="button button-secondary" onClick={() => onRestore(profile)} disabled={busy !== null}>{busy === `restore:${profile.id}` ? t("恢复中…") : t("恢复")}</button>
            </div>
          ))}
        </section>
      ) : null}

      <div className="privacy-lock"><strong>{t("共享状态提醒")}</strong><span>{t("Codex CLI 与 IDE 扩展共享一个活动账户。轮换只影响之后读取凭据的新会话；官方订阅凭据不会进入本地反代，也不会按请求自动换号。")}</span></div>
    </div>
  );
}

function OAuthView({ showNotice }: { showNotice: (notice: Notice) => void }) {
  const [activeTab, setActiveTab] = useState<OAuthTab>("login");
  const [snapshot, setSnapshot] = useState<CodexAccountSnapshot | null>(null);
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState<"login" | "refresh" | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [profiles, setProfiles] = useState<AuthProfilesSnapshot | null>(null);
  const [profilesLoading, setProfilesLoading] = useState(false);
  const [profileBusy, setProfileBusy] = useState<string | null>(null);
  const [profileLabel, setProfileLabel] = useState("");
  const refreshInFlight = useRef(false);
  const profilesAutoRequested = useRef(false);

  const refresh = useCallback(async (force = false, silent = false) => {
    if (refreshInFlight.current) return null;
    refreshInFlight.current = true;
    if (!silent) setBusy("refresh");
    try {
      const next = await invokeBackend<CodexAccountSnapshot>(COMMANDS.getCodexAccount, { refresh: force });
      setSnapshot(next);
      setError(null);
      return next;
    } catch (nextError: unknown) {
      const message = appError(nextError);
      setError(message);
      return null;
    } finally {
      setLoading(false);
      if (!silent) setBusy(null);
      refreshInFlight.current = false;
    }
  }, []);

  useEffect(() => {
    void refresh(false, true);
  }, [refresh]);

  useEffect(() => {
    if (!snapshot?.loginInProgress) return;
    const timer = window.setInterval(() => void refresh(true, true), 2_000);
    return () => window.clearInterval(timer);
  }, [refresh, snapshot?.loginInProgress]);

  const refreshProfiles = useCallback(async () => {
    setProfilesLoading(true);
    try {
      const next = await invokeBackend<AuthProfilesSnapshot>(COMMANDS.listAuthProfiles);
      setProfiles(next);
      return next;
    } catch (nextError: unknown) {
      showNotice({ tone: "error", message: tt`认证档案读取失败：${appError(nextError)}` });
      return null;
    } finally {
      setProfilesLoading(false);
    }
  }, [showNotice]);

  useEffect(() => {
    if (activeTab !== "credentials" || profiles || profilesAutoRequested.current) return;
    profilesAutoRequested.current = true;
    void refreshProfiles();
  }, [activeTab, profiles, refreshProfiles]);

  const finishProfileOperation = async (result: AuthProfileOperationResult) => {
    showNotice({ tone: result.changed ? "success" : "info", message: result.message });
    if (result.changed) {
      await refreshProfiles();
      await refresh(true, true);
    }
  };

  const importProfile = async () => {
    setProfileBusy("import");
    try {
      const result = await invokeBackend<AuthProfileOperationResult>(COMMANDS.importAuthProfile, { label: profileLabel.trim() });
      if (result.changed) setProfileLabel("");
      await finishProfileOperation(result);
    } catch (nextError: unknown) {
      showNotice({ tone: "error", message: tt`认证档案未导入：${appError(nextError)}` });
    } finally {
      setProfileBusy(null);
    }
  };

  const activateProfile = async (profile: AuthProfile) => {
    if (!profiles?.activeRevision) return;
    setProfileBusy(`activate:${profile.id}`);
    try {
      const result = await invokeBackend<AuthProfileOperationResult>(COMMANDS.activateAuthProfile, {
        profileId: profile.id,
        activeRevision: profiles.activeRevision,
      });
      await finishProfileOperation(result);
    } catch (nextError: unknown) {
      showNotice({ tone: "error", message: tt`账户未切换：${appError(nextError)}` });
      await refreshProfiles();
    } finally {
      setProfileBusy(null);
    }
  };

  const deleteProfile = async (profile: AuthProfile) => {
    setProfileBusy(`delete:${profile.id}`);
    try {
      const result = await invokeBackend<AuthProfileOperationResult>(COMMANDS.deleteAuthProfile, { profileId: profile.id });
      await finishProfileOperation(result);
    } catch (nextError: unknown) {
      showNotice({ tone: "error", message: tt`认证档案未删除：${appError(nextError)}` });
    } finally {
      setProfileBusy(null);
    }
  };

  const restoreProfile = async (profile: AuthProfile) => {
    setProfileBusy(`restore:${profile.id}`);
    try {
      const result = await invokeBackend<AuthProfileOperationResult>(COMMANDS.restoreAuthProfile, { profileId: profile.id });
      await finishProfileOperation(result);
    } catch (nextError: unknown) {
      showNotice({ tone: "error", message: tt`认证档案未恢复：${appError(nextError)}` });
    } finally {
      setProfileBusy(null);
    }
  };

  const startLogin = async () => {
    setBusy("login");
    try {
      const result = await invokeBackend<CodexLoginStartResult>(COMMANDS.startCodexLogin);
      setSnapshot((current) => current ? { ...current, loginInProgress: result.loginInProgress } : current);
      showNotice({
        tone: "info",
        message: result.started
          ? t("已启动官方 Codex 登录流程，请在系统浏览器中完成授权。")
          : t("已有 Codex 登录流程正在等待浏览器授权。"),
      });
      await refresh(true, true);
    } catch (nextError: unknown) {
      showNotice({ tone: "error", message: tt`登录流程未启动：${appError(nextError)}` });
    } finally {
      setBusy(null);
    }
  };

  const statusClass = snapshot?.state === "authenticated"
    ? "state-healthy"
    : snapshot?.state === "signed-out"
      ? "state-disabled"
      : "state-degraded";

  return (
    <div className="view-content oauth-view">
      <div className="oauth-tabs" role="tablist" aria-label={t("官方订阅功能")}>
        {oauthTabs.map((tab) => (
          <button key={tab.id} type="button" role="tab" aria-selected={activeTab === tab.id} className={activeTab === tab.id ? "is-active" : ""} onClick={() => setActiveTab(tab.id)}>
            {tab.label}
          </button>
        ))}
      </div>
      <SectionHeader
        title={activeTab === "login" ? t("Codex 官方订阅登录") : activeTab === "credentials" ? t("Codex 订阅账户") : t("Codex 额度与重置")}
        subtitle={activeTab === "credentials"
          ? t("导入的认证秘密只保存在应用专用 macOS Keychain；token、文件路径与原始 JSON 不会返回 WebView、SQLite 或日志。")
          : t("由官方 Codex CLI 与 App Server 完成认证和额度读取；OAuth token 不会返回 WebView、SQLite 或日志。")}
        action={<button type="button" className="button button-secondary" onClick={() => void refresh(true)} disabled={busy !== null}>{busy === "refresh" ? t("刷新中…") : t("刷新状态")}</button>}
      />

      {loading && !snapshot ? <LoadingState label={t("正在读取 Codex 账户状态…")} /> : null}
      {error && !snapshot ? <EmptyState title={t("账户状态暂不可用")} detail={error} action={<button type="button" className="button button-primary" onClick={() => void refresh(true)}>{t("重试")}</button>} /> : null}
      {error && snapshot ? <p className="oauth-cache-notice" role="status">{tt`刷新失败：${error}；当前仍显示${accountCacheLabel(snapshot)}。`}</p> : null}

      {snapshot && activeTab === "login" ? (
        <section className="panel oauth-login-card" aria-labelledby="codex-oauth-heading">
          <div className="oauth-provider-heading">
            <span className="oauth-provider-mark" aria-hidden="true">›_</span>
            <div><h2 id="codex-oauth-heading">Codex OAuth</h2><p>{t("系统浏览器会打开 OpenAI 登录页，回调、token 刷新与凭据存储全部由官方 Codex 处理。")}</p></div>
            <span className={`state-pill ${statusClass}`}>{accountStateLabel(snapshot)}</span>
          </div>
          <dl className="oauth-summary">
            <div><dt>{t("当前方式")}</dt><dd>{authMethodLabel(snapshot.authMethod)}</dd></div>
            <div><dt>{t("账号")}</dt><dd>{snapshot.email ?? "unavailable"}</dd></div>
            <div><dt>{t("套餐")}</dt><dd>{snapshot.planType ?? "unavailable"}</dd></div>
            <div><dt>{t("状态采集时间")}</dt><dd>{formatDate(snapshot.checkedAt)}</dd></div>
            <div><dt>{t("本地记录")}</dt><dd>{accountCacheLabel(snapshot)}</dd></div>
            <div><dt>{t("上次确认")}</dt><dd>{formatDate(accountCachedAt(snapshot) ?? snapshot.checkedAt)}</dd></div>
          </dl>
          <p className="capability-message">{t(snapshot.message)}</p>
          <button type="button" className="button button-primary oauth-login-button" onClick={() => void startLogin()} disabled={busy !== null || snapshot.loginInProgress}>
            {snapshot.loginInProgress ? t("等待浏览器授权…") : busy === "login" ? t("启动中…") : snapshot.authenticated ? t("重新登录") : t("开始登录")}
          </button>
          <p className="oauth-footnote">{t("登录可能改变 Codex CLI 与 IDE 扩展共享的当前账户。认证档案的导入、删除与切换只会在“订阅账户”页由你显式触发。")}</p>
        </section>
      ) : null}

      {activeTab === "credentials" ? <AuthProfilesPanel snapshot={profiles} loading={profilesLoading} busy={profileBusy} label={profileLabel} onLabelChange={setProfileLabel} onRefresh={() => void refreshProfiles()} onImport={() => void importProfile()} onActivate={(profile) => void activateProfile(profile)} onDelete={(profile) => void deleteProfile(profile)} onRestore={(profile) => void restoreProfile(profile)} /> : null}

      {snapshot && activeTab === "quota" ? (
        <div className="quota-stack">
          <div className="quota-summary-row">
            <span>{tt`${snapshot.rateLimits.length} 个可展示额度桶`}</span>
            <span>{tt`可用重置次数：${snapshot.availableResetCredits ?? "unavailable"}`}</span>
            <span>{tt`采集时间：${formatDate(snapshot.checkedAt)}`}</span>
          </div>
          {!snapshot.rateLimitsAvailable ? <EmptyState title={t("额度不可用")} detail={t("当前认证方式或 Codex 服务没有返回 ChatGPT 额度；不会用本地 token 消耗估算填充。")} /> : null}
          {snapshot.rateLimitsAvailable && snapshot.rateLimits.length === 0 ? <EmptyState title={t("没有可展示的额度窗口")} detail={t("官方服务返回了额度响应，但没有可识别的窗口。")} /> : null}
          {snapshot.rateLimits.map((bucket) => (
            <section key={bucket.limitId} className="panel quota-card">
              <div className="panel-title-row"><div><h2>{bucket.limitName ?? bucket.limitId}</h2><p>{bucket.planType ?? snapshot.planType ?? t("套餐 unavailable")} · {bucket.limitId}</p></div></div>
              {bucket.primary ? <QuotaWindow window={bucket.primary} label={t("主要额度")} /> : null}
              {bucket.secondary ? <QuotaWindow window={bucket.secondary} label={t("次要额度")} /> : null}
              {!bucket.primary && !bucket.secondary ? <p className="capability-message">{t("该额度桶没有可识别的时间窗口。")}</p> : null}
            </section>
          ))}
          <p className="oauth-footnote">{t("这里展示的是官方 ChatGPT Codex 额度窗口，不是 OpenAI Platform API 费率、API 等价成本或本地活动记录估算。")}</p>
        </div>
      ) : null}
    </div>
  );
}

function SettingsView({
  settings,
  capability,
  updateStatus,
  updateBusy,
  updateMessage,
  onSave,
  onProbe,
  onCheckUpdate,
  onInstallUpdate,
  saving,
}: {
  settings: AppSettings;
  capability: BootstrapPayload["capability"];
  updateStatus: AppUpdateStatus | null;
  updateBusy: "check" | "install" | null;
  updateMessage: { tone: "success" | "error"; text: string } | null;
  onSave: (next: AppSettings) => void;
  onProbe: () => void;
  onCheckUpdate: () => void;
  onInstallUpdate: () => void;
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
      <SectionHeader title={t("设置")} subtitle={t("所有路径设置仅在本机使用。授权根目录之外的 AGENTS 文件写入会被后端拒绝。")} />
      <div className="settings-grid">
        <form className="panel settings-form" onSubmit={submit}>
          <div className="panel-title-row"><div><h2>{t("本地采集")}</h2><p>{t("读取本机 Codex 产生的受支持元数据来源。")}</p></div></div>
          <label><span>{t("Codex home（每行一个）")}</span><textarea value={draft.codexHomes.join("\n")} onChange={(event) => setDraft({ ...draft, codexHomes: event.target.value.split("\n").map((value) => value.trim()).filter(Boolean) })} /></label>
          <label><span>{t("授权项目根目录（每行一个）")}</span><textarea value={draft.authorizedRoots.join("\n")} onChange={(event) => setDraft({ ...draft, authorizedRoots: event.target.value.split("\n").map((value) => value.trim()).filter(Boolean) })} /></label>
          <div className="settings-inline"><label><span>{t("保留天数")}</span><input type="number" min="1" max="3650" value={draft.retentionDays} onChange={(event) => setDraft({ ...draft, retentionDays: Math.max(1, Number(event.target.value)) })} /></label><label><span>{t("自动检查间隔（小时）")}</span><input type="number" min={MIN_UPDATE_CHECK_INTERVAL_HOURS} max={MAX_UPDATE_CHECK_INTERVAL_HOURS} value={draft.updateCheckIntervalHours} onChange={(event) => setDraft({ ...draft, updateCheckIntervalHours: normalizeUpdateCheckIntervalHours(Number(event.target.value)) })} /></label><label className="checkbox-row"><input type="checkbox" checked={draft.telemetryEnabled} onChange={(event) => setDraft({ ...draft, telemetryEnabled: event.target.checked })} /><span>{t("启用本机 OTel receiver")}</span></label></div>
          <p className="settings-disclaimer">{t("自动检查只读取固定 GitHub Release 元数据，不会下载或安装更新。")}</p>
          <p className="settings-disclaimer">{t("此开关只控制本应用的 loopback receiver；它不会修改 Codex 配置。要产生 OTel 数据，需由你自行在 Codex 配置中明确启用并指向本机接收器。")}</p>
          <div className="otel-config-box">
            <div className="editor-actions">
              <span className="muted">{t("配置片段包含本次运行的专用 token，只在你点击后显示。")}</span>
              <button className="button button-secondary" type="button" onClick={() => void revealOtelConfig()} disabled={!settings.telemetryEnabled}>{t("显示 OTel 配置")}</button>
            </div>
            {otelConfig ? <><dl className="capability-list"><div><dt>Endpoint</dt><dd><code>{otelConfig.endpoint}</code></dd></div><div><dt>CA certificate</dt><dd><code>{otelConfig.certificatePath}</code></dd></div><div><dt>Auth header</dt><dd><code>{otelConfig.headerName}</code></dd></div></dl><textarea className="otel-config-snippet" readOnly spellCheck={false} value={otelConfig.configSnippet} aria-label={t("Codex OTel 配置片段")} /></> : null}
            {otelConfigError ? <p className="capability-message">{otelConfigError}</p> : null}
          </div>
          <div className="privacy-lock"><strong>{t("隐私锁定")}</strong><span>{t("metadataOnly = true，消息正文、Authorization、Cookie、API Key 与完整环境变量不会保存。")}</span></div>
          <div className="editor-actions"><span className="muted">{tt`价格目录版本：${draft.priceCatalogVersion}`}</span><button className="button button-primary" type="submit" disabled={saving}>{saving ? t("保存中…") : t("保存设置")}</button></div>
        </form>
        <div className="settings-side-stack">
          <section className="panel capability-panel" aria-labelledby="capability-heading">
            <div className="panel-title-row"><div><h2 id="capability-heading">{t("CLI Schema 兼容性")}</h2><p>{t("用于诊断本机 Codex CLI 能否生成 App Server Schema；它不是采集源，不影响 rollout、账户读取或代理网关。")}</p></div><button type="button" className="button button-secondary" onClick={onProbe} disabled={saving}>{saving ? t("检查中…") : t("检查兼容性")}</button></div>
            {capability ? <dl className="capability-list"><div><dt>{t("可用性")}</dt><dd>{capability.available ? t("可用") : "unavailable"}</dd></div><div><dt>{t("版本")}</dt><dd>{capability.version ?? "unavailable"}</dd></div><div><dt>Schema SHA-256</dt><dd><code>{capability.schemaSha256 ?? "unavailable"}</code></dd></div><div><dt>{t("上次探测")}</dt><dd>{formatDate(capability.checkedAt)}</dd></div><div><dt>{t("可执行文件")}</dt><dd><code>{capability.executablePath}</code></dd></div></dl> : <EmptyState title={t("尚未检查 CLI Schema")} detail={t("未检查不代表系统故障；需要诊断 CLI 兼容性时再手动检查。")} />}
            {capability?.message ? <p className="capability-message">{t(capability.message)}</p> : null}
          </section>
          <section className={`panel update-panel${updateStatus?.available ? " update-panel-available" : ""}`} aria-labelledby="update-heading">
            <div className="panel-title-row"><div><h2 id="update-heading">{t("应用更新")}</h2><p>{t("按你设置的间隔读取固定 GitHub Releases 元数据；不会自动下载或安装。")}</p></div><button type="button" className="button button-secondary" onClick={onCheckUpdate} disabled={updateBusy !== null}>{updateBusy === "check" ? t("检查中…") : t("检查更新")}</button></div>
            {updateStatus ? <>
              {updateStatus.available ? <div className="update-available-banner" role="status"><strong>{tt`发现新版本 ${updateStatus.version ?? ""}`}</strong><span>{updateStatus.installable ? t("已准备好供你确认安装。") : t("这是已保存的检查结果；重新检查后才可安装。")}</span></div> : null}
              <dl className="capability-list"><div><dt>{t("当前版本")}</dt><dd>{updateStatus.currentVersion}</dd></div><div><dt>{t("可用版本")}</dt><dd>{updateStatus.version ?? t("已是最新")}</dd></div><div><dt>{t("最近检查")}</dt><dd>{formatDate(updateStatus.checkedAt)}</dd></div>{updateStatus.date ? <div><dt>{t("发布日期")}</dt><dd>{formatDate(updateStatus.date)}</dd></div> : null}</dl>
              {updateStatus.notes ? <p className="update-notes">{updateStatus.notes}</p> : null}
              {updateStatus.available && updateStatus.version ? <div className="update-actions"><span>{updateStatus.installable ? t("安装前会显示 macOS 原生确认框，并校验更新签名。") : t("这是上次检查的版本快照；请重新检查后再安装。")}</span><button type="button" className="button button-primary" onClick={updateStatus.installable ? onInstallUpdate : onCheckUpdate} disabled={updateBusy !== null}>{updateBusy === "install" ? t("安装中…") : updateBusy === "check" ? t("检查中…") : updateStatus.installable ? tt`安装 ${updateStatus.version}` : t("重新检查后安装")}</button></div> : null}
            </> : <p className="capability-message">{t("尚未检查。更新包来源、公钥和目标平台由桌面后端固定，网页层不能修改。")}</p>}
            {updateMessage ? <p className={`update-message update-message-${updateMessage.tone}`} role={updateMessage.tone === "error" ? "alert" : "status"}>{updateMessage.text}</p> : null}
          </section>
        </div>
      </div>
    </div>
  );
}

export function App() {
  const { locale } = useI18n();
  const navItems = getNavItems();
  const [view, setView] = useState<View>("overview");
  const [payload, setPayload] = useState<BootstrapPayload | null>(null);
  const [updateStatus, setUpdateStatus] = useState<AppUpdateStatus | null>(null);
  const [updateBusy, setUpdateBusy] = useState<"check" | "install" | null>(null);
  const [updateMessage, setUpdateMessage] = useState<{ tone: "success" | "error"; text: string } | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<Notice | null>(null);
  const [saving, setSaving] = useState(false);
  const [activityRefreshRevision, setActivityRefreshRevision] = useState(0);
  const [activityFullRefreshRevision, setActivityFullRefreshRevision] = useState(0);
  const bootstrapInFlight = useRef(false);
  const bootstrapPending = useRef<boolean | null>(null);
  const dashboardInFlight = useRef(false);
  const dashboardPending = useRef(false);
  const activityRescanInFlight = useRef(false);
  const updateCheckInFlight = useRef(false);
  const updateStatusRef = useRef<AppUpdateStatus | null>(null);
  const lastUpdateCheckAttemptAt = useRef<string | null>(null);
  const activeViewRef = useRef<View>(view);
  const previousViewRef = useRef<View>(view);

  activeViewRef.current = view;

  const showGatewayNotice = useCallback((tone: Notice["tone"], message: string) => {
    setNotice({ tone, message });
  }, []);

  const setCurrentUpdateStatus = useCallback((status: AppUpdateStatus | null) => {
    updateStatusRef.current = status;
    setUpdateStatus(status);
    setPayload((current) => current ? { ...current, updateStatus: status } : current);
  }, []);

  const setUpdateCheckLastAttemptAt = useCallback((attemptedAt: string | null) => {
    lastUpdateCheckAttemptAt.current = attemptedAt;
    setPayload((current) => current ? { ...current, updateCheckLastAttemptAt: attemptedAt } : current);
  }, []);

  const checkForUpdate = useCallback(async (automatic = false): Promise<boolean> => {
    if (updateCheckInFlight.current) return false;
    updateCheckInFlight.current = true;
    setUpdateBusy("check");
    setUpdateMessage(null);
    setUpdateCheckLastAttemptAt(new Date().toISOString());
    try {
      const status = await invokeBackend<AppUpdateStatus>(COMMANDS.checkForUpdate);
      setUpdateCheckLastAttemptAt(status.checkedAt);
      setCurrentUpdateStatus(status);
      if (!status.available && !automatic) setUpdateMessage({ tone: "success", text: t("当前已是最新版本。") });
      return true;
    } catch (nextError: unknown) {
      setUpdateMessage({ tone: "error", text: tt`${automatic ? "自动检查失败" : "检查更新失败"}：${appError(nextError)}` });
      return false;
    } finally {
      updateCheckInFlight.current = false;
      setUpdateBusy(null);
    }
  }, [setCurrentUpdateStatus, setUpdateCheckLastAttemptAt]);

  const installUpdate = useCallback(async () => {
    const status = updateStatusRef.current;
    if (!status?.available || !status.installable || !status.version) return;
    setUpdateBusy("install");
    setUpdateMessage(null);
    try {
      const result = await invokeBackend<UpdateInstallResult>(COMMANDS.installPendingUpdate, { expectedVersion: status.version });
      setUpdateMessage(result.accepted
        ? { tone: "success", text: t("更新已安装；桌面版将自动重启。") }
        : { tone: "success", text: t("已取消安装；再次安装前请重新检查更新。") });
      if (result.accepted) setCurrentUpdateStatus(null);
      else setCurrentUpdateStatus(markUpdateStatusUninstallable(status));
    } catch (nextError: unknown) {
      setCurrentUpdateStatus(markUpdateStatusUninstallable(status));
      setUpdateMessage({ tone: "error", text: tt`安装未完成：${appError(nextError)}` });
    } finally {
      setUpdateBusy(null);
    }
  }, [setCurrentUpdateStatus]);

  const refreshDashboard = useCallback(async () => {
    if (dashboardInFlight.current) {
      dashboardPending.current = true;
      return;
    }
    dashboardInFlight.current = true;
    try {
      const summary = await invokeBackend<NonNullable<BootstrapPayload["summary"]>>(COMMANDS.getDashboard);
      setPayload((current) => current ? {
        ...current,
        summary,
        activity: current.activity.total === null
          && current.activity.revision === summary.activityRevision
          ? { ...current.activity, total: summary.records }
          : current.activity,
      } : current);
    } catch {
      // The shell and activity list remain usable if the expensive aggregate
      // is temporarily unavailable. A later refresh retries it.
    } finally {
      const pending = dashboardPending.current;
      dashboardPending.current = false;
      dashboardInFlight.current = false;
      if (pending) void refreshDashboard();
    }
  }, []);

  const refreshBootstrap = useCallback(async (rescan = false) => {
    if (bootstrapInFlight.current) {
      bootstrapPending.current = bootstrapPending.current === true || rescan;
      return;
    }
    bootstrapInFlight.current = true;
    setLoading(true);
    setError(null);
    try {
      if (rescan && isDesktopRuntime()) {
        await invokeBackend<SourceHealth[]>(COMMANDS.rescan);
      }
      const next = await invokeBackend<BootstrapPayload>(COMMANDS.bootstrap);
      setPayload(next);
      updateStatusRef.current = next.updateStatus;
      setUpdateStatus(next.updateStatus);
      lastUpdateCheckAttemptAt.current = next.updateCheckLastAttemptAt;
    } catch (nextError: unknown) {
      setError(appError(nextError));
    } finally {
      const pendingRescan = bootstrapPending.current;
      bootstrapPending.current = null;
      bootstrapInFlight.current = false;
      setLoading(false);
      if (pendingRescan !== null) void refreshBootstrap(pendingRescan);
    }
  }, [refreshDashboard]);

  useEffect(() => { void refreshBootstrap(); }, [refreshBootstrap]);

  useEffect(() => {
    if (!isDesktopRuntime() || !payload) return;
    const schedule = getUpdateCheckSchedule({
      now: Date.now(),
      status: updateStatus,
      intervalHours: payload.settings.updateCheckIntervalHours,
      lastAttemptAt: lastUpdateCheckAttemptAt.current,
    });
    const timer = window.setTimeout(() => { void checkForUpdate(true); }, schedule.delayMs);
    return () => window.clearTimeout(timer);
  }, [checkForUpdate, payload, updateStatus]);

  const refreshLocalData = useCallback(async () => {
    if (activeViewRef.current === "activity" && isDesktopRuntime()) {
      if (activityRescanInFlight.current) return;
      activityRescanInFlight.current = true;
      setLoading(true);
      setError(null);
      try {
        await invokeBackend<SourceHealth[]>(COMMANDS.rescan);
        setActivityFullRefreshRevision((revision) => revision + 1);
      } catch (nextError: unknown) {
        setError(appError(nextError));
      } finally {
        activityRescanInFlight.current = false;
        setLoading(false);
      }
      return;
    }
    await refreshBootstrap(true);
  }, [refreshBootstrap]);

  useEffect(() => {
    if (!isDesktopRuntime()) return;
    let disposed = false;
    let unlisten: (() => void) | undefined;
    let pendingRefresh: number | undefined;
    const stopListening = (listener: (() => void) | undefined) => {
      if (!listener) return;
      try {
        void Promise.resolve(listener()).catch(() => undefined);
      } catch {
        // The window may already be gone during teardown or development reload.
      }
    };
    void import("@tauri-apps/api/event")
      .then(({ listen }) => listen("local-data-refreshed", () => {
        if (pendingRefresh !== undefined) return;
        pendingRefresh = window.setTimeout(() => {
          pendingRefresh = undefined;
          if (activeViewRef.current === "activity") {
            setActivityRefreshRevision((revision) => revision + 1);
          } else {
            void refreshBootstrap();
          }
        }, 2_000);
      }))
      .then((listener) => {
        if (disposed) stopListening(listener);
        else unlisten = listener;
      })
      .catch(() => undefined);
    return () => {
      disposed = true;
      if (pendingRefresh !== undefined) window.clearTimeout(pendingRefresh);
      stopListening(unlisten);
    };
  }, [refreshBootstrap]);

  const loadActivity = useCallback((query: ActivityQuery) => invokeBackend<ActivityPage>(COMMANDS.listActivity, { query }), []);

  useEffect(() => {
    const previous = previousViewRef.current;
    previousViewRef.current = view;
    if (!payload) return;
    if (payload.summary === null || (view === "overview" && previous !== "overview")) {
      void refreshDashboard();
    }
  }, [payload, refreshDashboard, view]);
  const updateSettings = async (next: AppSettings) => {
    setSaving(true);
    try {
      const saved = await invokeBackend<AppSettings>(COMMANDS.updateSettings, { settings: next });
      const sources = await invokeBackend<SourceHealth[]>(COMMANDS.listSources);
      setPayload((current) => current ? { ...current, settings: saved, sources } : current);
      setNotice({ tone: "success", message: t("设置已保存到本机，来源健康状态已刷新。") });
    } catch (nextError: unknown) {
      setNotice({ tone: "error", message: tt`设置未保存：${appError(nextError)}` });
    } finally {
      setSaving(false);
    }
  };
  const probe = async () => {
    setSaving(true);
    try {
      const capability = await invokeBackend<BootstrapPayload["capability"]>(COMMANDS.probeCodex);
      setPayload((current) => current ? { ...current, capability } : current);
      setNotice({ tone: "success", message: t("CLI Schema 兼容性检查完成。") });
    } catch (nextError: unknown) {
      setNotice({ tone: "error", message: tt`探测未完成：${appError(nextError)}` });
    } finally {
      setSaving(false);
    }
  };

  const shellLabel = isDesktopRuntime() ? t("本地桌面模式") : t("浏览器演示模式");
  const buildInfo = payload?.buildInfo ?? null;
  const buildLabel = buildInfo ? tt`v${buildInfo.version} · 构建 ${formatBuildTime(buildInfo.buildTime)}` : t("版本 unavailable");
  const projects = payload?.projects ?? [];
  const current = useMemo(() => {
    if (!payload) return null;
    switch (view) {
      case "overview": return <Overview payload={payload} onNavigate={setView} />;
      case "activity": return <ActivityView initial={payload.activity} projects={projects} onLoad={loadActivity} refreshRevision={activityRefreshRevision} fullRefreshRevision={activityFullRefreshRevision} />;
      case "projects": return <ProjectsView projects={projects} authorizedRoots={payload.settings.authorizedRoots} onProjectsChange={(next) => setPayload((old) => old ? { ...old, projects: next } : old)} showNotice={setNotice} />;
      case "config": return <CodexConfigView onNotice={showGatewayNotice} onOpenOfficialSubscription={() => setView("oauth")} />;
      case "gateway": return <CodexGatewayView onNotice={showGatewayNotice} />;
      case "oauth": return <OAuthView showNotice={setNotice} />;
      case "pricing": return <PricingView rules={payload.pricingRules} />;
      case "settings": return <SettingsView settings={payload.settings} capability={payload.capability} updateStatus={updateStatus} updateBusy={updateBusy} updateMessage={updateMessage} onSave={(next) => void updateSettings(next)} onProbe={() => void probe()} onCheckUpdate={() => void checkForUpdate()} onInstallUpdate={() => void installUpdate()} saving={saving} />;
    }
  }, [activityFullRefreshRevision, activityRefreshRevision, checkForUpdate, installUpdate, loadActivity, locale, payload, projects, saving, showGatewayNotice, updateBusy, updateMessage, updateStatus, view]);

  return (
    <div className="app-shell">
      <aside className="sidebar">
        <div className="brand"><span className="brand-mark">CM</span><div><strong>Codex Manager</strong><small>{t("本地桌面管理器")}</small></div></div>
        <nav className="side-nav" aria-label={t("主导航")}>
          {navItems.map((item) => <button type="button" key={item.id} className={view === item.id ? "is-active" : ""} onClick={() => setView(item.id)}><span>{item.label}{item.id === "settings" && updateStatus?.available ? <><span className="update-nav-dot" aria-hidden="true" /><span className="sr-only">{t("，有新版本可用")}</span></> : null}</span><small>{item.helper}</small></button>)}
        </nav>
        <div className="sidebar-foot"><span className="runtime-mark" aria-hidden="true" /><span>{shellLabel}</span><small>{buildLabel}</small><small>{t("网关默认停止；不自动修改 Codex 配置。")}</small></div>
      </aside>
      <main className="main-area">
        <header className="topbar"><div><span className="topbar-crumb">{t("工作台")}</span><span className="topbar-separator">/</span><span className="topbar-view">{navItems.find((item) => item.id === view)?.label}</span><small className="topbar-build">{shellLabel} · {buildLabel}</small></div><div className="topbar-actions"><LanguageSwitcher /><button type="button" className="button button-secondary" onClick={() => void refreshLocalData()} disabled={loading}>{loading ? t("刷新中…") : t("刷新本地数据")}</button></div></header>
        {notice ? <InlineNotice notice={notice} onClose={() => setNotice(null)} /> : null}
        {loading && !payload ? <LoadingState /> : null}
        {error && !payload ? <div className="fatal-error"><EmptyState title={t("无法连接本地管理服务")} detail={error} action={<button type="button" className="button button-primary" onClick={() => void refreshBootstrap()}>{t("重试")}</button>} /></div> : null}
        {current}
      </main>
    </div>
  );
}
