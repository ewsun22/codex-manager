export type MetricConfidence =
  | "observed"
  | "codex-reported"
  | "derived"
  | "configured"
  | "unavailable";

export interface TokenUsage {
  inputTokens: number;
  cachedInputTokens: number;
  cacheWriteInputTokens: number;
  outputTokens: number;
  reasoningOutputTokens: number;
  totalTokens: number;
}

export interface SessionRecord {
  sessionId: string;
  threadId?: string;
  cliVersion?: string;
  cwd?: string;
  modelProvider?: string;
  sourceKind?: string;
  startedAt?: string;
}

export interface TurnRecord {
  turnId: string;
  sessionId: string;
  model?: string;
  reasoningEffort?: string;
  modelProvider?: string;
  cwd?: string;
  startedAt?: string;
  completedAt?: string;
  durationMs?: number;
  timeToFirstTokenMs?: number;
  status: "running" | "completed" | "failed" | "aborted" | "unknown";
  timingConfidence: MetricConfidence;
  usage: TokenUsage;
  usageConfidence: MetricConfidence;
  modelCallCount: number;
}

export interface ModelCallRecord {
  eventKey: string;
  sessionId: string;
  turnId: string;
  ordinal?: number;
  timestamp?: string;
  model?: string;
  reasoningEffort?: string;
  modelProvider?: string;
  usage: TokenUsage;
  cumulativeUsage: TokenUsage;
  usageConfidence: MetricConfidence;
}

export interface TimelineItem {
  eventKey: string;
  sessionId: string;
  turnId?: string;
  ordinal?: number;
  timestamp?: string;
  itemId?: string;
  itemType: string;
  role?: string;
  phase?: string;
  toolName?: string;
  contentUtf8Bytes?: number;
  contentPersisted: false;
}

export interface ParserDiagnostic {
  code:
    | "invalid_json"
    | "invalid_envelope"
    | "missing_session"
    | "missing_turn"
    | "invalid_usage"
    | "usage_overflow"
    | "usage_call_out_of_range"
    | "reported_usage_mismatch"
    | "non_monotonic_usage"
    | "unknown_event";
  severity: "info" | "warning" | "error";
  ordinal?: number;
  message: string;
}

export interface RolloutSnapshot {
  session?: SessionRecord;
  turns: TurnRecord[];
  modelCalls: ModelCallRecord[];
  timeline: TimelineItem[];
  diagnostics: ParserDiagnostic[];
  unhandledEventCounts: Record<string, number>;
  ignoredDuplicateEvents: number;
  ignoredDuplicateUsageSnapshots: number;
}
