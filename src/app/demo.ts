import type {
  ActivityPage,
  ActivityQuery,
  ActivityRecord,
  AgentsChain,
  AgentsFileSnapshot,
  AgentsRevision,
  AppSettings,
  AppUpdateStatus,
  AuthProfileOperationResult,
  AuthProfilesSnapshot,
  BootstrapPayload,
  CodexCapability,
  CodexAccountSnapshot,
  CodexConfigApplyResult,
  CodexConfigPreview,
  CodexConfigProfile,
  CodexConfigSnapshot,
  CodexGatewayStatus,
  CodexLoginStartResult,
  CodexProviderProfile,
  ProxyAuthImportOutcome,
  ProxyAuthProfile,
  ProxyAuthProfileList,
  CreateAgentsFileInput,
  DashboardSummary,
  PricingRule,
  ProjectSummary,
  SaveAgentsInput,
  SaveAgentsResult,
  SourceHealth,
  UpdateInstallResult,
  SaveCodexProviderInput,
} from "../shared/contracts.ts";
import { COMMANDS } from "../shared/contracts.ts";

const observedAt = "2026-08-27T10:32:00.000Z";
const projectPath = "/Users/example/Projects/signal-catalog";
const infraProjectPath = "/Users/example/Projects/infra-scripts";
const globalAgentsPath = "/Users/example/.codex/AGENTS.md";
const projectAgentsPath = `${projectPath}/AGENTS.md`;
const activityRevision = "demo-activity-60";

const summary: DashboardSummary = {
  activityRevision,
  rangeLabel: "最近 24 小时",
  records: 60,
  modelCalls: 60,
  successful: 45,
  failed: 15,
  inputTokens: 7_642_600,
  cachedInputTokens: 7_422_180,
  cacheWriteInputTokens: 75_904,
  outputTokens: 10_912,
  reasoningOutputTokens: 1_305,
  equivalentCostUsd: 7.8642,
  pricedCalls: 57,
  totalCostCalls: 60,
  pricedTokens: 7_645_920,
  observedTokens: 7_730_721,
  unpricedCalls: 3,
};

const activitySeed: ActivityRecord[] = [
  {
    id: "turn-00042",
    activityKind: "turn",
    occurredAt: "2026-08-27T10:31:45.000Z",
    projectName: "signal-catalog",
    projectPath,
    sessionId: "019…2f6",
    turnId: "turn-42",
    model: { value: "gpt-5.6-sol", source: "rollout", confidence: "high" },
    effort: { value: "high", source: "rollout", confidence: "high" },
    provider: { value: "codex", source: "rollout", confidence: "high" },
    endpoint: { value: null, source: "unavailable", confidence: "unavailable" },
    inputTokens: { value: 168_020, source: "rollout", confidence: "high" },
    cachedInputTokens: { value: 167_168, source: "derived", confidence: "medium" },
    cacheWriteInputTokens: { value: 852, source: "rollout", confidence: "high" },
    outputTokens: { value: 233, source: "rollout", confidence: "high" },
    reasoningOutputTokens: { value: null, source: "unavailable", confidence: "unavailable" },
    totalTokens: { value: 168_253, source: "derived", confidence: "high" },
    cacheHitRatio: { value: 0.9949, source: "derived", confidence: "medium" },
    equivalentCostUsd: { value: 0.1219, source: "configured", confidence: "medium" },
    durationMs: { value: 10_749, source: "rollout", confidence: "high" },
    firstVisibleOutputMs: { value: 3_766, source: "rollout", confidence: "medium" },
    responseBytes: { value: null, source: "unavailable", confidence: "unavailable" },
    result: "success",
    parentTurnResult: null,
    timingScope: "turn",
    modelCallCount: 2,
    statusCode: 200,
  },
  {
    id: "turn-00041",
    activityKind: "turn",
    occurredAt: "2026-08-27T10:30:52.000Z",
    projectName: "infra-scripts",
    projectPath: "/Users/example/Projects/infra-scripts",
    sessionId: "019…a6e",
    turnId: "turn-11",
    model: { value: "gpt-5.6-terra", source: "rollout", confidence: "high" },
    effort: { value: "medium", source: "rollout", confidence: "high" },
    provider: { value: "relay.example.net", source: "rollout", confidence: "low" },
    endpoint: { value: "https://relay.example.test/v1/responses", source: "server", confidence: "high" },
    inputTokens: { value: 121_088, source: "rollout", confidence: "high" },
    cachedInputTokens: { value: 119_552, source: "derived", confidence: "medium" },
    cacheWriteInputTokens: { value: 1_536, source: "rollout", confidence: "high" },
    outputTokens: { value: 158, source: "rollout", confidence: "high" },
    reasoningOutputTokens: { value: 45, source: "rollout", confidence: "medium" },
    totalTokens: { value: 121_246, source: "derived", confidence: "high" },
    cacheHitRatio: { value: 0.9873, source: "derived", confidence: "medium" },
    equivalentCostUsd: { value: 0.0884, source: "configured", confidence: "medium" },
    durationMs: { value: 9_936, source: "rollout", confidence: "high" },
    firstVisibleOutputMs: { value: 4_368, source: "rollout", confidence: "medium" },
    responseBytes: { value: 8_491, source: "server", confidence: "medium" },
    result: "success",
    parentTurnResult: null,
    timingScope: "turn",
    modelCallCount: 1,
    statusCode: 200,
  },
  {
    id: "turn-00040",
    activityKind: "turn",
    occurredAt: "2026-08-27T10:29:16.000Z",
    projectName: "signal-catalog",
    projectPath,
    sessionId: "019…2f6",
    turnId: "turn-41",
    model: { value: "gpt-5.6-sol", source: "rollout", confidence: "high" },
    effort: { value: "xhigh", source: "rollout", confidence: "high" },
    provider: { value: "codex", source: "rollout", confidence: "high" },
    endpoint: { value: null, source: "unavailable", confidence: "unavailable" },
    inputTokens: { value: 0, source: "rollout", confidence: "high" },
    cachedInputTokens: { value: 0, source: "rollout", confidence: "high" },
    cacheWriteInputTokens: { value: 0, source: "rollout", confidence: "high" },
    outputTokens: { value: 0, source: "rollout", confidence: "high" },
    reasoningOutputTokens: { value: null, source: "unavailable", confidence: "unavailable" },
    totalTokens: { value: 0, source: "rollout", confidence: "high" },
    cacheHitRatio: { value: null, source: "unavailable", confidence: "unavailable" },
    equivalentCostUsd: { value: null, source: "unavailable", confidence: "unavailable" },
    durationMs: { value: 5_166, source: "rollout", confidence: "high" },
    firstVisibleOutputMs: { value: null, source: "unavailable", confidence: "unavailable" },
    responseBytes: { value: null, source: "unavailable", confidence: "unavailable" },
    result: "failure",
    parentTurnResult: null,
    timingScope: "turn",
    modelCallCount: 1,
    statusCode: 500,
  },
  {
    id: "turn-00039",
    activityKind: "turn",
    occurredAt: "2026-08-27T10:24:11.000Z",
    projectName: "marketing-site",
    projectPath: "/Users/example/Projects/marketing-site",
    sessionId: "019…b90",
    turnId: "turn-8",
    model: { value: "gpt-5.6-terra", source: "rollout", confidence: "high" },
    effort: { value: "medium", source: "rollout", confidence: "high" },
    provider: { value: "codex", source: "rollout", confidence: "high" },
    endpoint: { value: null, source: "unavailable", confidence: "unavailable" },
    inputTokens: { value: 161_290, source: "rollout", confidence: "high" },
    cachedInputTokens: { value: 158_976, source: "derived", confidence: "medium" },
    cacheWriteInputTokens: { value: 2_314, source: "rollout", confidence: "high" },
    outputTokens: { value: 204, source: "rollout", confidence: "high" },
    reasoningOutputTokens: { value: null, source: "unavailable", confidence: "unavailable" },
    totalTokens: { value: 161_494, source: "derived", confidence: "high" },
    cacheHitRatio: { value: 0.9857, source: "derived", confidence: "medium" },
    equivalentCostUsd: { value: 0.1172, source: "configured", confidence: "medium" },
    durationMs: { value: 9_717, source: "rollout", confidence: "high" },
    firstVisibleOutputMs: { value: 4_426, source: "rollout", confidence: "medium" },
    responseBytes: { value: 10_991, source: "server", confidence: "medium" },
    result: "success",
    parentTurnResult: null,
    timingScope: "turn",
    modelCallCount: 3,
    statusCode: 200,
  },
];

const activity: ActivityRecord[] = Array.from({ length: 60 }, (_, index) => {
  const seed = activitySeed[index % activitySeed.length];
  const sequence = 60 - index;
  const offsetMinutes = index * 3;
  const input = seed.inputTokens.value === null ? null : seed.inputTokens.value + (index * 211);
  const cached = seed.cachedInputTokens.value === null ? null : Math.min(seed.cachedInputTokens.value + (index * 183), input ?? 0);
  const output = seed.outputTokens.value === null ? null : seed.outputTokens.value + (index % 9);
  const cacheWrite = seed.cacheWriteInputTokens.value === null ? null : seed.cacheWriteInputTokens.value + (index % 5) * 64;
  const isOtelRequest = index % 11 === 6;
  const isModelCall = !isOtelRequest && index % 3 === 1;
  return {
    ...seed,
    id: `turn-${String(sequence).padStart(5, "0")}`,
    occurredAt: new Date(Date.parse(observedAt) - offsetMinutes * 60_000).toISOString(),
    sessionId: `019-demo-${String(Math.ceil(sequence / 4)).padStart(3, "0")}`,
    turnId: `turn-${sequence}`,
    inputTokens: { ...seed.inputTokens, value: input },
    cachedInputTokens: { ...seed.cachedInputTokens, value: cached },
    cacheWriteInputTokens: { ...seed.cacheWriteInputTokens, value: cacheWrite },
    outputTokens: { ...seed.outputTokens, value: output },
    totalTokens: { ...seed.totalTokens, value: input === null || output === null ? null : input + output },
    activityKind: isOtelRequest ? "otelRequest" : isModelCall ? "modelCall" : "turn",
    result: isModelCall ? "accounted" : seed.result,
    parentTurnResult: isModelCall ? seed.result : null,
    timingScope: isOtelRequest ? "request" : isModelCall ? "unavailable" : "turn",
    modelCallCount: isOtelRequest ? 0 : isModelCall ? 1 : seed.modelCallCount,
    firstVisibleOutputMs: (isModelCall || isOtelRequest)
      ? { value: null, source: "unavailable", confidence: "unavailable" }
      : seed.firstVisibleOutputMs,
    durationMs: isModelCall
      ? { value: null, source: "unavailable", confidence: "unavailable" }
      : seed.durationMs,
    statusCode: isModelCall ? null : seed.statusCode,
  };
});

const projects: ProjectSummary[] = [
  {
    id: "project-signal",
    name: "signal-catalog",
    canonicalPath: projectPath,
    source: "observed",
    exists: true,
    isGit: true,
    worktree: false,
    lastSeenAt: observedAt,
    lastConversationAt: observedAt,
    agentsFileCount: 2,
    hasAgentsFile: true,
  },
  {
    id: "project-infra",
    name: "infra-scripts",
    canonicalPath: infraProjectPath,
    source: "observed",
    exists: true,
    isGit: true,
    worktree: true,
    lastSeenAt: "2026-08-27T10:30:52.000Z",
    lastConversationAt: "2026-08-27T10:30:52.000Z",
    agentsFileCount: 1,
    hasAgentsFile: true,
  },
  {
    id: "project-marketing",
    name: "marketing-site",
    canonicalPath: "/Users/example/Projects/marketing-site",
    source: "manual",
    exists: true,
    isGit: true,
    worktree: false,
    lastSeenAt: null,
    lastConversationAt: null,
    agentsFileCount: 0,
    hasAgentsFile: false,
  },
];

let sources: SourceHealth[] = [
  {
    id: "rollout-main",
    kind: "rollout",
    label: "本地 rollout JSONL",
    state: "healthy",
    lastObservedAt: observedAt,
    lagMs: 1_200,
    codexVersion: "0.150.0-alpha.8",
    schemaSha256: null,
    unparsedEvents: 0,
    message: "仅采集元数据，消息正文未入库。",
  },
  {
    id: "app-server",
    kind: "app-server",
    label: "CLI Schema 兼容性（非采集源）",
    state: "healthy",
    lastObservedAt: "2026-08-27T10:00:00.000Z",
    lagMs: null,
    codexVersion: "0.150.0-alpha.8",
    schemaSha256: "cc0e…a4f2",
    unparsedEvents: 0,
    message: "已验证本地 CLI Schema 兼容性；未附着到正在运行的私有进程，也不影响 rollout 主采集。",
  },
  {
    id: "otel-local",
    kind: "otel",
    label: "本地 OTel receiver",
    state: "disabled",
    lastObservedAt: null,
    lagMs: null,
    codexVersion: null,
    schemaSha256: null,
    unparsedEvents: 0,
    message: "需要用户在 Codex 配置中明确启用。",
  },
];

const pricingRules: PricingRule[] = [
  {
    id: "openai-2026-08-27-sol",
    provider: "OpenAI",
    modelPattern: "gpt-5.6-sol",
    effectiveFrom: "2026-08-27",
    currency: "USD",
    inputPerMillion: 4,
    cachedInputPerMillion: 0.4,
    cacheWriteInputPerMillion: 5,
    outputPerMillion: 20,
    sourceUrl: null,
    sourceLabel: "openai-2026-08-27.json",
    isUserOverride: false,
  },
  {
    id: "openai-2026-08-27-terra",
    provider: "OpenAI",
    modelPattern: "gpt-5.6-terra",
    effectiveFrom: "2026-08-27",
    currency: "USD",
    inputPerMillion: 2,
    cachedInputPerMillion: 0.2,
    cacheWriteInputPerMillion: 2.5,
    outputPerMillion: 12,
    sourceUrl: null,
    sourceLabel: "openai-2026-08-27.json",
    isUserOverride: false,
  },
  {
    id: "openai-2026-08-27-luna",
    provider: "OpenAI",
    modelPattern: "gpt-5.6-luna",
    effectiveFrom: "2026-08-27",
    currency: "USD",
    inputPerMillion: 0.2,
    cachedInputPerMillion: 0.02,
    cacheWriteInputPerMillion: 0.25,
    outputPerMillion: 1.2,
    sourceUrl: null,
    sourceLabel: "openai-2026-08-27.json",
    isUserOverride: false,
  },
];

let settings: AppSettings = {
  codexHomes: ["/Users/example/.codex"],
  authorizedRoots: ["/Users/example/Projects"],
  retentionDays: 30,
  updateCheckIntervalHours: 12,
  metadataOnly: true,
  telemetryEnabled: false,
  priceCatalogVersion: "openai-2026-08-27",
};

const capability: CodexCapability = {
  executablePath: "/Applications/ChatGPT.app/.../codex",
  version: "0.150.0-alpha.8",
  schemaSha256: "cc0ef07d…a4f2",
  checkedAt: observedAt,
  available: true,
  message: "演示数据：schema 指纹仅用于兼容性提示。",
};

const account: CodexAccountSnapshot = {
  state: "authenticated",
  authenticated: true,
  authMethod: "chatgpt",
  email: "developer@example.test",
  planType: "pro",
  requiresOpenaiAuth: true,
  loginInProgress: false,
  rateLimitsAvailable: true,
  rateLimits: [
    {
      limitId: "codex",
      limitName: "Codex",
      planType: "pro",
      primary: { usedPercent: 38, windowDurationMins: 300, resetsAt: "2026-08-28T16:00:00.000Z" },
      secondary: { usedPercent: 57, windowDurationMins: 10_080, resetsAt: "2026-09-03T16:00:00.000Z" },
    },
    {
      limitId: "codex_other",
      limitName: "其他 Codex 模型",
      planType: "pro",
      primary: { usedPercent: 12, windowDurationMins: 1_440, resetsAt: "2026-08-29T16:00:00.000Z" },
      secondary: null,
    },
  ],
  availableResetCredits: 1,
  source: "cache",
  stale: false,
  cachedAt: observedAt,
  checkedAt: observedAt,
  message: "演示数据：账户与额度来自官方 Codex App Server，凭据不会进入界面。",
};

let authProfiles: AuthProfilesSnapshot = {
  supported: true,
  activationAvailable: true,
  activeProfileId: "profile-demo-primary",
  activeRevision: "revision-demo-1",
  profiles: [
    { id: "profile-demo-primary", label: "个人 Pro", isActive: true, createdAt: "2026-08-27T08:00:00.000Z", updatedAt: observedAt, deletedAt: null },
    { id: "profile-demo-team", label: "团队账户", isActive: false, createdAt: "2026-08-27T09:00:00.000Z", updatedAt: "2026-08-27T09:30:00.000Z", deletedAt: null },
  ],
  deletedRetentionDays: 30,
  message: "演示模式：档案秘密位于应用专用 macOS Keychain，活动账户使用 file 模式 auth.json。",
};

let proxyAuthProfiles: ProxyAuthProfile[] = [
  {
    id: "proxy-auth-demo-codex",
    label: "Codex 个人账户",
    provider: "codex",
    enabled: true,
    createdAt: "2026-08-27T08:00:00.000Z",
    updatedAt: observedAt,
    lastCheckpointAt: null,
    state: "ready",
    errorCode: null,
  },
  {
    id: "proxy-auth-demo-claude",
    label: "Claude 备用账户",
    provider: "claude",
    enabled: false,
    createdAt: "2026-08-27T09:00:00.000Z",
    updatedAt: observedAt,
    lastCheckpointAt: null,
    state: "disabled",
    errorCode: null,
  },
];

let codexConfigSnapshot: CodexConfigSnapshot = {
  profiles: [
    {
      id: "official-direct",
      name: "官方直连",
      kind: "official-direct",
      baseUrl: null,
      model: null,
      reasoningEffort: null,
      secretRef: null,
      isActive: true,
      isVerified: true,
      createdAt: observedAt,
      updatedAt: observedAt,
    },
    {
      id: "local-cliproxy",
      name: "本地 CLIProxyAPI",
      kind: "local-cliproxy",
      baseUrl: "http://127.0.0.1:47653/v1",
      model: "gpt-5.6-sol",
      reasoningEffort: "high",
      secretRef: "local-proxy-client",
      isActive: false,
      isVerified: false,
      createdAt: observedAt,
      updatedAt: observedAt,
    },
  ],
  activeProfileId: "official-direct",
  activeRevision: "demo-config-revision-1",
  state: "official-direct",
  message: null,
};

let codexProviders: CodexProviderProfile[] = [
  {
    id: "00000000-0000-4000-8000-000000000001",
    name: "OpenAI 中转",
    baseUrl: "https://relay.example.test/v1",
    model: "gpt-5.6-sol",
    reasoningEffort: "high",
    hasApiKey: true,
    createdAt: "2026-08-27T08:30:00.000Z",
    updatedAt: observedAt,
  },
  {
    id: "00000000-0000-4000-8000-000000000002",
    name: "本机兼容上游",
    baseUrl: "http://127.0.0.1:8317/v1",
    model: "gpt-5.6-terra",
    reasoningEffort: "medium",
    hasApiKey: true,
    createdAt: "2026-08-27T09:00:00.000Z",
    updatedAt: observedAt,
  },
];

let codexGatewayStatus: CodexGatewayStatus = {
  state: "stopped",
  source: "none",
  installed: true,
  coreVersion: "7.2.145",
  latestVersion: "7.2.145",
  updateAvailable: false,
  installing: false,
  processId: null,
  coreSource: "已审核固定基线: router-for-me/CLIProxyAPI v7.2.145",
  providerId: null,
  providerName: null,
  endpoint: null,
  startedAt: null,
  requests: null,
  failed: null,
  inFlight: null,
  lastError: null,
  port: 47653,
};

let currentAgentsContent = `# signal-catalog agent guidance\n\n## 范围\n\n- 优先修复用户路径，不执行未授权的部署。\n- 数据采集默认仅保留元数据。\n\n## 验收\n\n- 修改后运行相关测试，并报告实际结果。\n`;
const createdProjectAgents = new Set<string>([projectAgentsPath, `${infraProjectPath}/AGENTS.md`]);
let revisionCounter = 2;
let revisions: AgentsRevision[] = [
  {
    id: "revision-02",
    path: projectAgentsPath,
    createdAt: "2026-08-27T09:10:00.000Z",
    beforeSha256: "8e4b…b1b5",
    afterSha256: "f2d3…2aa8",
    byteLength: 198,
  },
  {
    id: "revision-01",
    path: projectAgentsPath,
    createdAt: "2026-08-26T16:12:00.000Z",
    beforeSha256: "2c56…bfd0",
    afterSha256: "8e4b…b1b5",
    byteLength: 176,
  },
];

function agentsSnapshot(path = projectAgentsPath): AgentsFileSnapshot {
  const isGlobal = path === globalAgentsPath;
  const content = isGlobal ? "# 全局规则\n\n- 保持说明简洁。\n" : currentAgentsContent;
  return {
    path,
    relativePath: isGlobal ? "~/.codex/AGENTS.md" : "AGENTS.md",
    kind: isGlobal ? "global" : "project",
    precedence: isGlobal ? 10 : 40,
    effective: true,
    overridden: false,
    sha256: "f2d3…2aa8",
    mtimeMs: 1_787_999_000_000,
    sizeBytes: content.length,
    writable: !isGlobal,
    content,
    newline: "lf",
  };
}

function chainFor(project: ProjectSummary): AgentsChain {
  const candidatePath = `${project.canonicalPath}/AGENTS.md`;
  const hasProjectAgents = createdProjectAgents.has(candidatePath);
  return {
    project,
    selectedCwd: project.canonicalPath,
    files: [
      agentsSnapshot(globalAgentsPath),
      ...(hasProjectAgents ? [{
        ...agentsSnapshot(candidatePath),
        path: candidatePath,
        relativePath: "AGENTS.md",
        effective: true,
        overridden: false,
      }] : []),
    ],
    effectivePaths: [globalAgentsPath, ...(hasProjectAgents ? [candidatePath] : [])],
    warnings:
      hasProjectAgents
        ? ["演示模式：保存操作只写入内存，不会改动本机文件。"]
        : ["未发现项目级 AGENTS.md。缺失值不按空文件处理。"],
    maxBytes: 32_768,
  };
}

function filterActivity(query: ActivityQuery | undefined): ActivityRecord[] {
  const search = query?.search?.trim().toLocaleLowerCase();
  return activity.filter((item) => {
    const view = query?.view ?? "turns";
    if (view === "turns" && item.activityKind !== "turn") return false;
    if (view === "modelCalls" && item.activityKind === "turn") return false;
    if (query?.projectPath && item.projectPath !== query.projectPath) return false;
    if (query?.model && item.model.value !== query.model) return false;
    if (query?.effort && item.effort.value !== query.effort) return false;
    if (query?.result && item.result !== query.result) return false;
    if (!search) return true;
    return [item.projectName, item.model.value, item.provider.value, item.sessionId]
      .filter(Boolean)
      .join(" ")
      .toLocaleLowerCase()
      .includes(search);
  });
}

export async function invokeDemo<T>(command: string, args: Record<string, unknown> = {}): Promise<T> {
  const query = args.query as ActivityQuery | undefined;
  const page = (items: ActivityRecord[]): ActivityPage => {
    const limit = Math.max(1, Math.min(query?.limit ?? 50, 100));
    const offset = Number.parseInt(query?.cursor ?? "0", 10);
    const safeOffset = Number.isSafeInteger(offset) && offset >= 0 ? offset : 0;
    const nextOffset = safeOffset + limit;
    return {
      items: items.slice(safeOffset, nextOffset),
      nextCursor: nextOffset < items.length ? String(nextOffset) : null,
      total: query?.refreshOnly ? null : items.length,
      revision: activityRevision,
    };
  };

  switch (command) {
    case COMMANDS.bootstrap:
      return {
        summary,
        activity: page(activity),
        projects,
        sources,
        pricingRules,
        settings,
        capability,
        buildInfo: {
          version: "0.6.1",
          buildTime: observedAt,
          commitSha: null,
        },
        updateStatus: null,
        updateCheckLastAttemptAt: null,
      } satisfies BootstrapPayload as T;
    case COMMANDS.getDashboard:
      return summary as T;
    case COMMANDS.listActivity:
      if (query?.revisionOnly) {
        return {
          items: [],
          nextCursor: null,
          total: null,
          revision: activityRevision,
        } satisfies ActivityPage as T;
      }
      return page(filterActivity(query)) as T;
    case COMMANDS.listProjects:
    case COMMANDS.discoverProjects:
      return projects as T;
    case COMMANDS.getAgentsChain: {
      const selected = projects.find((project) => project.canonicalPath === args.projectPath) ?? projects[0];
      return chainFor(selected) as T;
    }
    case COMMANDS.openAgentsFile: {
      const path = String(args.path ?? projectAgentsPath);
      return agentsSnapshot(path) as T;
    }
    case COMMANDS.createAgentsFile: {
      const input = args.input as CreateAgentsFileInput;
      const path = `${input.projectPath}/${input.fileName}`;
      if (input.fileName !== "AGENTS.md" || createdProjectAgents.has(path)) {
        throw new Error("项目级 AGENTS.md 已存在，或文件名不受支持。");
      }
      createdProjectAgents.add(path);
      const project = projects.find((candidate) => candidate.canonicalPath === input.projectPath);
      if (project) {
        project.hasAgentsFile = true;
        project.agentsFileCount = Math.max(1, project.agentsFileCount + 1);
      }
      currentAgentsContent = input.content;
      revisionCounter += 1;
      const revisionId = `revision-${String(revisionCounter).padStart(2, "0")}`;
      revisions = [{ id: revisionId, path, createdAt: new Date().toISOString(), beforeSha256: "new-file", afterSha256: "f2d3…2aa8", byteLength: input.content.length }, ...revisions];
      return { snapshot: agentsSnapshot(path), revisionId } satisfies SaveAgentsResult as T;
    }
    case COMMANDS.saveAgentsFile: {
      const input = args.input as SaveAgentsInput;
      const beforeSha256 = agentsSnapshot(input.path).sha256;
      currentAgentsContent = input.content;
      revisionCounter += 1;
      const revisionId = `revision-${String(revisionCounter).padStart(2, "0")}`;
      revisions = [
        {
          id: revisionId,
          path: input.path,
          createdAt: new Date().toISOString(),
          beforeSha256,
          afterSha256: "f2d3…2aa8",
          byteLength: input.content.length,
        },
        ...revisions,
      ];
      return { snapshot: agentsSnapshot(input.path), revisionId } satisfies SaveAgentsResult as T;
    }
    case COMMANDS.listAgentsRevisions:
      return String(args.path ?? "") === globalAgentsPath ? [] as T : revisions as T;
    case COMMANDS.restoreAgentsRevision:
      currentAgentsContent = "# signal-catalog agent guidance\n\n- 已从本地 revision 演示恢复。\n";
      return { snapshot: agentsSnapshot(), revisionId: "revision-restored" } satisfies SaveAgentsResult as T;
    case COMMANDS.listSources:
      return sources as T;
    case COMMANDS.listPricingRules:
      return pricingRules as T;
    case COMMANDS.getSettings:
      return settings as T;
    case COMMANDS.getOtelConfig:
      if (!settings.telemetryEnabled) throw new Error("OTel receiver 尚未启用；请先保存设置。");
      return {
        endpoint: "https://localhost:54321",
        certificatePath: "/Users/example/Library/Application Support/Codex Manager/otel/ca.pem",
        headerName: "x-codex-manager-token",
        configSnippet: "[otel]\n# 桌面演示模式不显示真实凭据\n",
      } as T;
    case COMMANDS.updateSettings:
      settings = args.settings as AppSettings;
      sources = sources.map((source) => source.kind === "otel" ? {
        ...source,
        state: settings.telemetryEnabled ? "healthy" : "disabled",
        lastObservedAt: settings.telemetryEnabled ? new Date().toISOString() : null,
        message: settings.telemetryEnabled ? "演示模式：本机 loopback receiver 已启用；仍需用户自行配置 Codex exporter。" : "未启用，不监听端口。",
      } : source);
      return settings as T;
    case COMMANDS.rescan:
      return { started: true, observedAt: new Date().toISOString() } as T;
    case COMMANDS.probeCodex:
      return capability as T;
    case COMMANDS.checkForUpdate:
      return {
        currentVersion: "0.1.0",
        available: true,
        installable: true,
        version: "0.2.1",
        date: "2026-08-27T10:00:00.000Z",
        notes: "演示更新：新增 GitHub Releases 在线更新与签名校验。",
        checkedAt: new Date().toISOString(),
      } satisfies AppUpdateStatus as T;
    case COMMANDS.installPendingUpdate:
      if (args.expectedVersion !== "0.2.1") throw new Error("更新版本已变化，请重新检查后再安装。");
      return { accepted: true } satisfies UpdateInstallResult as T;
    case COMMANDS.getCodexAccount:
      if (args.refresh === true) {
        const refreshedAt = new Date().toISOString();
        return {
          ...account,
          source: "live",
          stale: false,
          cachedAt: refreshedAt,
          checkedAt: refreshedAt,
        } as T;
      }
      return structuredClone(account) as T;
    case COMMANDS.startCodexLogin:
      return { started: true, loginInProgress: true } satisfies CodexLoginStartResult as T;
    case COMMANDS.listAuthProfiles:
      return structuredClone(authProfiles) as T;
    case COMMANDS.importAuthProfile: {
      const now = new Date().toISOString();
      const id = `profile-demo-${authProfiles.profiles.length + 1}`;
      authProfiles.profiles.push({ id, label: String(args.label || "新认证档案"), isActive: false, createdAt: now, updatedAt: now, deletedAt: null });
      return { changed: true, message: "演示模式：认证档案已导入。" } satisfies AuthProfileOperationResult as T;
    }
    case COMMANDS.activateAuthProfile: {
      const id = String(args.profileId ?? "");
      const target = authProfiles.profiles.find((profile) => profile.id === id);
      if (!target || target.deletedAt !== null) throw new Error("认证档案不存在或已删除。");
      authProfiles.profiles = authProfiles.profiles.map((profile) => ({ ...profile, isActive: profile.id === id }));
      authProfiles.activeProfileId = id;
      authProfiles.activeRevision = `revision-demo-${Date.now()}`;
      return { changed: true, message: "演示模式：已切换当前认证档案。" } satisfies AuthProfileOperationResult as T;
    }
    case COMMANDS.deleteAuthProfile: {
      const id = String(args.profileId ?? "");
      const target = authProfiles.profiles.find((profile) => profile.id === id);
      if (!target || target.deletedAt !== null) throw new Error("认证档案不存在或已删除。");
      if (target.isActive || authProfiles.activeProfileId === id) throw new Error("不能删除当前生效的认证档案。");
      const availableCount = authProfiles.profiles.filter((profile) => profile.deletedAt === null).length;
      if (availableCount <= 1) throw new Error("至少必须保留一个可用认证档案。");
      const deletedAt = new Date().toISOString();
      authProfiles.profiles = authProfiles.profiles.map((profile) => profile.id === id ? { ...profile, isActive: false, deletedAt } : profile);
      return { changed: true, message: "演示模式：档案已移入回收站。" } satisfies AuthProfileOperationResult as T;
    }
    case COMMANDS.restoreAuthProfile: {
      const id = String(args.profileId ?? "");
      const target = authProfiles.profiles.find((profile) => profile.id === id);
      if (!target) throw new Error("认证档案不存在。");
      if (target.deletedAt === null) throw new Error("认证档案尚未删除。");
      authProfiles.profiles = authProfiles.profiles.map((profile) => profile.id === id ? { ...profile, deletedAt: null } : profile);
      return { changed: true, message: "演示模式：档案已恢复。" } satisfies AuthProfileOperationResult as T;
    }
    case COMMANDS.getCodexConfigSnapshot:
      return structuredClone(codexConfigSnapshot) as T;
    case COMMANDS.saveCodexConfigProfile: {
      const input = args.profile as CodexConfigProfile;
      const now = new Date().toISOString();
      const existing = codexConfigSnapshot.profiles.find((profile) => profile.id === input.id);
      const profile: CodexConfigProfile = {
        ...input,
        isActive: existing?.isActive ?? false,
        isVerified: existing?.isVerified ?? false,
        createdAt: existing?.createdAt ?? now,
        updatedAt: now,
      };
      codexConfigSnapshot.profiles = existing
        ? codexConfigSnapshot.profiles.map((item) => item.id === profile.id ? profile : item)
        : [...codexConfigSnapshot.profiles, profile];
      return structuredClone(profile) as T;
    }
    case COMMANDS.deleteCodexConfigProfile: {
      const profileId = String(args.profileId ?? "");
      const target = codexConfigSnapshot.profiles.find((profile) => profile.id === profileId);
      if (target?.isActive) throw new Error("当前正在使用该配置档案。");
      const before = codexConfigSnapshot.profiles.length;
      codexConfigSnapshot.profiles = codexConfigSnapshot.profiles.filter((profile) => profile.id !== profileId);
      return (codexConfigSnapshot.profiles.length !== before) as T;
    }
    case COMMANDS.previewCodexConfigProfile: {
      const profileId = String(args.profileId ?? "");
      const profile = codexConfigSnapshot.profiles.find((item) => item.id === profileId);
      if (!profile) throw new Error("Codex 配置档案不存在。");
      const configSnippet = profile.kind === "official-direct"
        ? "# 恢复到 Codex Manager 接管前的配置。\n"
        : `model = ${JSON.stringify(profile.model)}\nmodel_provider = ${JSON.stringify(`codex-manager-${profile.id}`)}\n\n[model_providers.${JSON.stringify(`codex-manager-${profile.id}`)}]\nbase_url = ${JSON.stringify(profile.baseUrl)}\nwire_api = "responses"\nauth = { command = "/Applications/Codex Manager.app/Contents/MacOS/codex-manager-desktop", args = ["--codex-manager-credential", "${profile.secretRef}"] }\n`;
      return {
        profileId,
        expectedRevision: codexConfigSnapshot.activeRevision,
        configSnippet,
        managedFields: ["model", "model_provider", "model_providers.<managed>"],
      } satisfies CodexConfigPreview as T;
    }
    case COMMANDS.applyCodexConfigProfile: {
      const profileId = String(args.profileId ?? "");
      if (!codexConfigSnapshot.profiles.some((profile) => profile.id === profileId)) {
        throw new Error("Codex 配置档案不存在。");
      }
      codexConfigSnapshot.profiles = codexConfigSnapshot.profiles.map((profile) => ({
        ...profile,
        isActive: profile.id === profileId,
        isVerified: profile.id === profileId ? true : profile.isVerified,
      }));
      codexConfigSnapshot.activeProfileId = profileId;
      codexConfigSnapshot.activeRevision = `demo-config-revision-${Date.now()}`;
      codexConfigSnapshot.state = profileId === "official-direct" ? "official-direct" : "active";
      return {
        changed: true,
        state: codexConfigSnapshot.state,
        activeProfileId: profileId,
        activeRevision: codexConfigSnapshot.activeRevision,
        verified: true,
        message: "演示模式：已写入并复核 Codex 配置。",
      } satisfies CodexConfigApplyResult as T;
    }
    case COMMANDS.restoreCodexConfig: {
      codexConfigSnapshot.profiles = codexConfigSnapshot.profiles.map((profile) => ({
        ...profile,
        isActive: profile.id === "official-direct",
        isVerified: profile.id === "official-direct" ? true : profile.isVerified,
      }));
      codexConfigSnapshot.activeProfileId = "official-direct";
      codexConfigSnapshot.activeRevision = `demo-config-revision-${Date.now()}`;
      codexConfigSnapshot.state = "official-direct";
      return {
        changed: true,
        state: "official-direct",
        activeProfileId: "official-direct",
        activeRevision: codexConfigSnapshot.activeRevision,
        verified: true,
        message: "演示模式：已恢复接管前的配置。",
      } satisfies CodexConfigApplyResult as T;
    }
    case COMMANDS.listProxyAuthProfiles:
      return {
        profiles: structuredClone(proxyAuthProfiles.filter((profile) => Boolean(args.includeDeleted) || profile.state !== "deleted")),
      } satisfies ProxyAuthProfileList as T;
    case COMMANDS.importProxyAuthProfile: {
      const now = new Date().toISOString();
      const profile: ProxyAuthProfile = {
        id: `proxy-auth-demo-${proxyAuthProfiles.length + 1}`,
        label: String(args.label || "新 OAuth 档案"),
        provider: "codex",
        enabled: true,
        createdAt: now,
        updatedAt: now,
        lastCheckpointAt: null,
        state: "ready",
        errorCode: null,
      };
      proxyAuthProfiles.push(profile);
      return { profile, action: "created" } satisfies ProxyAuthImportOutcome as T;
    }
    case COMMANDS.setProxyAuthProfileEnabled: {
      const profileId = String(args.profileId ?? "");
      const enabled = Boolean(args.enabled);
      const target = proxyAuthProfiles.find((profile) => profile.id === profileId && profile.state !== "deleted");
      if (!target) throw new Error("OAuth 档案不存在。");
      Object.assign(target, { enabled, state: enabled ? "ready" : "disabled", updatedAt: new Date().toISOString() });
      return structuredClone(target) as T;
    }
    case COMMANDS.deleteProxyAuthProfile: {
      const profileId = String(args.profileId ?? "");
      const target = proxyAuthProfiles.find((profile) => profile.id === profileId && profile.state !== "deleted");
      if (!target) throw new Error("OAuth 档案不存在。");
      Object.assign(target, { enabled: false, state: "deleted", updatedAt: new Date().toISOString() });
      return structuredClone(target) as T;
    }
    case COMMANDS.restoreProxyAuthProfile: {
      const profileId = String(args.profileId ?? "");
      const target = proxyAuthProfiles.find((profile) => profile.id === profileId && profile.state === "deleted");
      if (!target) throw new Error("OAuth 档案尚未删除。");
      Object.assign(target, { enabled: false, state: "disabled", updatedAt: new Date().toISOString() });
      return structuredClone(target) as T;
    }
    case COMMANDS.listCodexProviders:
      return structuredClone(codexProviders) as T;
    case COMMANDS.saveCodexProvider: {
      const input = args.input as SaveCodexProviderInput;
      const now = new Date().toISOString();
      const existing = input.id ? codexProviders.find((provider) => provider.id === input.id) : undefined;
      const profile: CodexProviderProfile = {
        id: existing?.id ?? crypto.randomUUID(),
        name: input.name,
        baseUrl: input.baseUrl.replace(/\/$/, ""),
        model: input.model,
        reasoningEffort: input.reasoningEffort,
        hasApiKey: Boolean(input.apiKey?.trim()) || existing?.hasApiKey === true,
        createdAt: existing?.createdAt ?? now,
        updatedAt: now,
      };
      codexProviders = existing
        ? codexProviders.map((provider) => provider.id === existing.id ? profile : provider)
        : [...codexProviders, profile];
      return structuredClone(profile) as T;
    }
    case COMMANDS.deleteCodexProvider: {
      const providerId = String(args.providerId ?? "");
      if (codexConfigSnapshot.profiles.some((profile) => profile.secretRef === `external-provider:${providerId}`)) throw new Error("该供应商仍被 Codex 配置档案引用，请先删除或改用其他供应商。");
      codexProviders = codexProviders.filter((provider) => provider.id !== providerId);
      return structuredClone(codexProviders) as T;
    }
    case COMMANDS.getCodexGatewayStatus:
      return structuredClone(codexGatewayStatus) as T;
    case COMMANDS.checkLatestCliproxyCore:
      codexGatewayStatus = {
        ...codexGatewayStatus,
        latestVersion: "7.2.145",
        updateAvailable: codexGatewayStatus.coreVersion !== "7.2.145",
      };
      return structuredClone(codexGatewayStatus) as T;
    case COMMANDS.installLatestCliproxyCore:
      if (codexGatewayStatus.state === "running") throw new Error("请先关闭反代再更新内核。");
      codexGatewayStatus = {
        ...codexGatewayStatus,
        installed: true,
        installing: false,
        coreVersion: codexGatewayStatus.latestVersion ?? "7.2.145",
        latestVersion: codexGatewayStatus.latestVersion ?? "7.2.145",
        updateAvailable: false,
      };
      return structuredClone(codexGatewayStatus) as T;
    case COMMANDS.updateCodexGatewayPort:
      if (codexGatewayStatus.state === "running") throw new Error("网关运行中不能修改端口。");
      codexGatewayStatus = { ...codexGatewayStatus, port: Number(args.port) };
      return structuredClone(codexGatewayStatus) as T;
    case COMMANDS.startCodexGateway: {
      if (!proxyAuthProfiles.some((profile) => profile.enabled && profile.state === "ready")) {
        throw new Error("没有已启用的 OAuth 档案。");
      }
      codexGatewayStatus = {
        ...codexGatewayStatus,
        state: "running",
        source: "oauth-pool",
        providerId: null,
        providerName: "CLIProxyAPI OAuth 凭据池",
        endpoint: `http://127.0.0.1:${codexGatewayStatus.port}/v1`,
        startedAt: new Date().toISOString(),
        processId: 59317,
        lastError: null,
      };
      return structuredClone(codexGatewayStatus) as T;
    }
    case COMMANDS.stopCodexGateway:
      codexGatewayStatus = {
        ...codexGatewayStatus,
        state: "stopped",
        source: "none",
        providerId: null,
        providerName: null,
        endpoint: null,
        startedAt: null,
        processId: null,
        inFlight: null,
      };
      return structuredClone(codexGatewayStatus) as T;
    default:
      throw new Error(`演示适配器不支持命令：${command}`);
  }
}
