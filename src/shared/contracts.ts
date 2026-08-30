export type MetricSource =
  | "rollout"
  | "derived"
  | "configured"
  | "server"
  | "manual"
  | "unavailable";

export type MetricConfidence = "high" | "medium" | "low" | "unavailable";

export interface MetricValue<T> {
  value: T | null;
  source: MetricSource;
  confidence: MetricConfidence;
}

export interface DashboardSummary {
  activityRevision: string;
  rangeLabel: string;
  /** Completed or otherwise terminal task summaries in the selected range. */
  records: number;
  /** Canonical model-call records represented by the task summaries. */
  modelCalls: number;
  successful: number;
  failed: number;
  inputTokens: number | null;
  cachedInputTokens: number | null;
  cacheWriteInputTokens: number | null;
  outputTokens: number | null;
  reasoningOutputTokens: number | null;
  equivalentCostUsd: number | null;
  /** Calls included in the equivalent API-cost estimate. */
  pricedCalls: number;
  /** All canonical calls considered for equivalent API-cost coverage. */
  totalCostCalls: number;
  pricedTokens: number | null;
  observedTokens: number | null;
  unpricedCalls: number;
}

export type ActivityResult = "success" | "failure" | "running" | "unknown" | "accounted" | "unobserved";
export type ActivityKind = "turn" | "modelCall" | "otelRequest";
export type ActivityTimingScope = "turn" | "request" | "unavailable";

export interface ActivityRecord {
  id: string;
  /** Determines the row's semantic unit; it is never inferred by the UI. */
  activityKind: ActivityKind;
  occurredAt: string;
  projectName: string | null;
  projectPath: string | null;
  sessionId: string;
  turnId: string | null;
  model: MetricValue<string>;
  effort: MetricValue<string>;
  provider: MetricValue<string>;
  endpoint: MetricValue<string>;
  inputTokens: MetricValue<number>;
  cachedInputTokens: MetricValue<number>;
  cacheWriteInputTokens: MetricValue<number>;
  outputTokens: MetricValue<number>;
  reasoningOutputTokens: MetricValue<number>;
  totalTokens: MetricValue<number>;
  cacheHitRatio: MetricValue<number>;
  equivalentCostUsd: MetricValue<number>;
  durationMs: MetricValue<number>;
  firstVisibleOutputMs: MetricValue<number>;
  responseBytes: MetricValue<number>;
  result: ActivityResult;
  /** The associated task result for a model call or OTel request, if observed. */
  parentTurnResult: ActivityResult | null;
  /** States whether timing values apply to the task, request, or neither. */
  timingScope: ActivityTimingScope;
  /** Canonical model-call count represented by this row. */
  modelCallCount: number;
  statusCode: number | null;
}

export interface ActivityQuery {
  cursor?: string | null;
  limit?: number;
  projectPath?: string | null;
  model?: string | null;
  effort?: string | null;
  result?: ActivityResult | null;
  search?: string | null;
  view?: "turns" | "modelCalls";
  refreshOnly?: boolean;
  revisionOnly?: boolean;
}

export interface ActivityPage {
  items: ActivityRecord[];
  nextCursor: string | null;
  total: number | null;
  revision: string;
}

export interface SourceHealth {
  id: string;
  kind: "rollout" | "app-server" | "otel";
  label: string;
  state: "healthy" | "degraded" | "disabled" | "unavailable" | "unchecked";
  lastObservedAt: string | null;
  lagMs: number | null;
  codexVersion: string | null;
  schemaSha256: string | null;
  unparsedEvents: number;
  message: string | null;
}

export interface PricingRule {
  id: string;
  provider: string;
  modelPattern: string;
  effectiveFrom: string;
  currency: "USD";
  inputPerMillion: number;
  cachedInputPerMillion: number;
  cacheWriteInputPerMillion: number;
  outputPerMillion: number;
  sourceUrl: string | null;
  sourceLabel: string;
  isUserOverride: boolean;
}

export interface ProjectSummary {
  id: string;
  name: string;
  canonicalPath: string;
  source: "observed" | "manual";
  exists: boolean;
  isGit: boolean;
  worktree: boolean;
  lastSeenAt: string | null;
  /** The newest observed conversation timestamp under this project. */
  lastConversationAt: string | null;
  agentsFileCount: number;
  hasAgentsFile: boolean;
}

export type AgentsFileKind = "global" | "project" | "nested" | "fallback";

export interface AgentsFileSummary {
  path: string;
  relativePath: string;
  kind: AgentsFileKind;
  precedence: number;
  effective: boolean;
  overridden: boolean;
  sha256: string;
  mtimeMs: number;
  sizeBytes: number;
  writable: boolean;
}

export interface AgentsChain {
  project: ProjectSummary;
  selectedCwd: string;
  files: AgentsFileSummary[];
  effectivePaths: string[];
  warnings: string[];
  maxBytes: number;
}

export interface AgentsFileSnapshot extends AgentsFileSummary {
  content: string;
  newline: "lf" | "crlf";
}

export interface SaveAgentsInput {
  path: string;
  authorizedRoot: string;
  content: string;
  expectedSha256: string;
  expectedMtimeMs: number;
}

export interface CreateAgentsFileInput {
  projectPath: string;
  authorizedRoot: string;
  content: string;
  fileName: "AGENTS.md";
}

export interface SaveAgentsResult {
  snapshot: AgentsFileSnapshot;
  revisionId: string;
}

export interface AgentsRevision {
  id: string;
  path: string;
  createdAt: string;
  beforeSha256: string;
  afterSha256: string;
  byteLength: number;
}

export interface CodexCapability {
  executablePath: string;
  version: string | null;
  schemaSha256: string | null;
  checkedAt: string;
  available: boolean;
  message: string | null;
}

export interface AppSettings {
  codexHomes: string[];
  authorizedRoots: string[];
  retentionDays: number;
  updateCheckIntervalHours: number;
  metadataOnly: true;
  telemetryEnabled: boolean;
  priceCatalogVersion: string;
}

export interface OtelConfig {
  endpoint: string;
  certificatePath: string;
  headerName: string;
  configSnippet: string;
}

export interface AppUpdateStatus {
  currentVersion: string;
  available: boolean;
  installable: boolean;
  version: string | null;
  date: string | null;
  notes: string | null;
  checkedAt: string;
}

export interface UpdateInstallResult {
  accepted: boolean;
}

export type CodexAccountState = "authenticated" | "signed-out" | "unavailable";
export type CodexAccountSource = "live" | "cache" | "stale" | "unavailable";

export interface CodexRateLimitWindow {
  usedPercent: number;
  windowDurationMins: number | null;
  resetsAt: string | null;
}

export interface CodexRateLimitBucket {
  limitId: string;
  limitName: string | null;
  planType: string | null;
  primary: CodexRateLimitWindow | null;
  secondary: CodexRateLimitWindow | null;
}

export interface CodexAccountSnapshot {
  state: CodexAccountState;
  authenticated: boolean;
  authMethod: string | null;
  email: string | null;
  planType: string | null;
  requiresOpenaiAuth: boolean | null;
  loginInProgress: boolean;
  rateLimitsAvailable: boolean;
  rateLimits: CodexRateLimitBucket[];
  availableResetCredits: number | null;
  checkedAt: string;
  /** Local provenance only; never derived from an OAuth response. */
  source: CodexAccountSource;
  /** True when a forced refresh failed or the saved snapshot is older than six hours. */
  stale: boolean;
  /** RFC 3339 time at which the sanitized Keychain cache was last written. */
  cachedAt: string | null;
  message: string;
}

export interface CodexLoginStartResult {
  started: boolean;
  loginInProgress: boolean;
}

export interface AuthProfile {
  id: string;
  label: string;
  isActive: boolean;
  createdAt: string;
  updatedAt: string;
  deletedAt: string | null;
}

export interface AuthProfilesSnapshot {
  supported: boolean;
  activationAvailable: boolean;
  activeProfileId: string | null;
  activeRevision: string | null;
  profiles: AuthProfile[];
  deletedRetentionDays: number;
  message: string;
}

export interface AuthProfileOperationResult {
  changed: boolean;
  message: string;
}

export type CodexReasoningEffort = "minimal" | "low" | "medium" | "high" | "xhigh";

export interface CodexProviderProfile {
  id: string;
  name: string;
  baseUrl: string;
  model: string;
  reasoningEffort: CodexReasoningEffort;
  hasApiKey: boolean;
  createdAt: string;
  updatedAt: string;
}

export interface SaveCodexProviderInput {
  id?: string;
  name: string;
  baseUrl: string;
  model: string;
  reasoningEffort: CodexReasoningEffort;
  /** Write-only. An empty value retains the existing Keychain secret. */
  apiKey?: string;
}

export type CodexGatewayState = "stopped" | "running" | "error";

export interface CodexGatewayStatus {
  state: CodexGatewayState;
  providerId: string | null;
  providerName: string | null;
  endpoint: string | null;
  startedAt: string | null;
  requests: number;
  failed: number;
  inFlight: number;
  lastError: string | null;
  port: number;
}

/** Explicit reveal result. configSnippet contains a local-only client bearer. */
export interface CodexGatewaySetup {
  port: number;
  endpoint: string;
  configSnippet: string;
}

export interface BootstrapPayload {
  summary: DashboardSummary | null;
  activity: ActivityPage;
  projects: ProjectSummary[];
  sources: SourceHealth[];
  pricingRules: PricingRule[];
  settings: AppSettings;
  capability: CodexCapability | null;
  updateStatus: AppUpdateStatus | null;
  updateCheckLastAttemptAt: string | null;
  buildInfo: BuildInfo;
}

export interface BuildInfo {
  version: string;
  /** Compile-time UTC build timestamp, not the process start time. */
  buildTime: string;
  /** Optional, sanitized short commit SHA supplied by the build environment. */
  commitSha: string | null;
}

export const COMMANDS = {
  bootstrap: "bootstrap",
  getDashboard: "get_dashboard",
  listActivity: "list_activity",
  listProjects: "list_projects",
  discoverProjects: "discover_projects",
  getAgentsChain: "get_agents_chain",
  openAgentsFile: "open_agents_file",
  createAgentsFile: "create_agents_file",
  saveAgentsFile: "save_agents_file",
  listAgentsRevisions: "list_agents_revisions",
  restoreAgentsRevision: "restore_agents_revision",
  listSources: "list_sources",
  listPricingRules: "list_pricing_rules",
  getSettings: "get_settings",
  getOtelConfig: "get_otel_config",
  updateSettings: "update_settings",
  rescan: "rescan",
  probeCodex: "probe_codex",
  checkForUpdate: "check_for_update",
  installPendingUpdate: "install_pending_update",
  getCodexAccount: "get_codex_account",
  startCodexLogin: "start_codex_login",
  listAuthProfiles: "list_auth_profiles",
  importAuthProfile: "import_auth_profile",
  activateAuthProfile: "activate_auth_profile",
  deleteAuthProfile: "delete_auth_profile",
  restoreAuthProfile: "restore_auth_profile",
  listCodexProviders: "list_codex_providers",
  saveCodexProvider: "save_codex_provider",
  deleteCodexProvider: "delete_codex_provider",
  getCodexGatewayStatus: "get_codex_gateway_status",
  updateCodexGatewayPort: "update_codex_gateway_port",
  startCodexGateway: "start_codex_gateway",
  stopCodexGateway: "stop_codex_gateway",
  revealCodexGatewaySetup: "reveal_codex_gateway_setup",
} as const;
