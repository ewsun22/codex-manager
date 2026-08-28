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
  rangeLabel: string;
  records: number;
  successful: number;
  failed: number;
  inputTokens: number | null;
  cachedInputTokens: number | null;
  cacheWriteInputTokens: number | null;
  outputTokens: number | null;
  reasoningOutputTokens: number | null;
  equivalentCostUsd: number | null;
}

export type ActivityResult = "success" | "failure" | "running" | "unknown";

export interface ActivityRecord {
  id: string;
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
}

export interface ActivityPage {
  items: ActivityRecord[];
  nextCursor: string | null;
  total: number;
}

export interface SourceHealth {
  id: string;
  kind: "rollout" | "app-server" | "otel";
  label: string;
  state: "healthy" | "degraded" | "disabled" | "unavailable";
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
  version: string | null;
  date: string | null;
  notes: string | null;
}

export interface UpdateInstallResult {
  accepted: boolean;
}

export type CodexAccountState = "authenticated" | "signed-out" | "unavailable";

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

export interface BootstrapPayload {
  summary: DashboardSummary;
  activity: ActivityPage;
  projects: ProjectSummary[];
  sources: SourceHealth[];
  pricingRules: PricingRule[];
  settings: AppSettings;
  capability: CodexCapability | null;
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
} as const;
