import type {
  MetricConfidence,
  ModelCallRecord,
  ParserDiagnostic,
  RolloutSnapshot,
  SessionRecord,
  TimelineItem,
  TokenUsage,
  TurnRecord,
} from "./types.ts";
import {
  ZERO_USAGE,
  addUsage,
  isMonotonicUsage,
  isSameUsage,
  isZeroUsage,
  isValidCallUsage,
  parseTokenUsage,
  subtractUsage,
} from "./usage.ts";

type JsonObject = Record<string, unknown>;

interface RolloutEnvelope {
  ordinal?: number;
  timestamp?: string;
  type: string;
  payload: JsonObject;
}

export class RolloutNormalizer {
  private session?: SessionRecord;
  private currentTurnId?: string;
  private readonly turns = new Map<string, TurnRecord>();
  private readonly modelCalls: ModelCallRecord[] = [];
  private readonly timeline: TimelineItem[] = [];
  private readonly diagnostics: ParserDiagnostic[] = [];
  private readonly unhandledEventCounts = new Map<string, number>();
  private readonly seenEventKeys = new Set<string>();
  private lastCumulativeUsage?: TokenUsage;
  private ignoredDuplicateEvents = 0;
  private ignoredDuplicateUsageSnapshots = 0;

  parseLine(line: string, sourcePosition?: number | string): void {
    let raw: unknown;
    try {
      raw = JSON.parse(line);
    } catch {
      this.diagnostics.push({
        code: "invalid_json",
        severity: "warning",
        message: "Skipped an invalid JSONL line.",
      });
      return;
    }

    const envelope = parseEnvelope(raw);
    if (!envelope) {
      this.diagnostics.push({
        code: "invalid_envelope",
        severity: "warning",
        message: "Skipped an event without the expected envelope fields.",
      });
      return;
    }

    if (envelope.type === "session_meta") {
      this.parseSessionMeta(envelope, sourcePosition);
      return;
    }

    if (!this.session) {
      this.diagnostics.push({
        code: "missing_session",
        severity: "warning",
        ordinal: envelope.ordinal,
        message: "Skipped an event that appeared before session metadata.",
      });
      return;
    }

    const eventKey = this.createEventKey(envelope, sourcePosition);
    if (this.seenEventKeys.has(eventKey)) {
      this.ignoredDuplicateEvents += 1;
      return;
    }
    this.seenEventKeys.add(eventKey);

    if (envelope.type === "turn_context") {
      this.parseTurnContext(envelope);
      return;
    }

    if (envelope.type === "response_item") {
      this.parseTimelineItem(envelope, eventKey);
      return;
    }

    if (envelope.type === "event_msg") {
      this.parseEventMessage(envelope, eventKey);
      return;
    }

    this.countUnhandledEvent(
      "top:" + safeEventTypeLabel(envelope.type),
    );
  }

  snapshot(): RolloutSnapshot {
    return {
      session: this.session ? { ...this.session } : undefined,
      turns: [...this.turns.values()].map(cloneTurn),
      modelCalls: this.modelCalls.map((call) => ({
        ...call,
        usage: { ...call.usage },
        cumulativeUsage: { ...call.cumulativeUsage },
      })),
      timeline: this.timeline.map((item) => ({ ...item })),
      diagnostics: this.diagnostics.map((diagnostic) => ({ ...diagnostic })),
      unhandledEventCounts: Object.fromEntries(this.unhandledEventCounts),
      ignoredDuplicateEvents: this.ignoredDuplicateEvents,
      ignoredDuplicateUsageSnapshots: this.ignoredDuplicateUsageSnapshots,
    };
  }

  private parseSessionMeta(
    envelope: RolloutEnvelope,
    sourcePosition?: number | string,
  ): void {
    const payload = envelope.payload;
    const sessionId =
      stringValue(payload.session_id) ??
      stringValue(payload.id) ??
      stringValue(payload.thread_id);

    if (!sessionId) {
      this.diagnostics.push({
        code: "invalid_envelope",
        severity: "error",
        ordinal: envelope.ordinal,
        message: "Session metadata did not include an identifier.",
      });
      return;
    }

    this.session = {
      sessionId,
      threadId:
        stringValue(payload.thread_id) ?? stringValue(payload.id) ?? sessionId,
      cliVersion: stringValue(payload.cli_version),
      cwd: stringValue(payload.cwd),
      modelProvider: stringValue(payload.model_provider),
      sourceKind: sourceKind(payload.source) ?? stringValue(payload.thread_source),
      startedAt: stringValue(payload.timestamp) ?? envelope.timestamp,
    };

    const eventKey = this.createEventKey(envelope, sourcePosition);
    if (this.seenEventKeys.has(eventKey)) {
      this.ignoredDuplicateEvents += 1;
    } else {
      this.seenEventKeys.add(eventKey);
    }
  }

  private parseTurnContext(envelope: RolloutEnvelope): void {
    const turnId =
      stringValue(envelope.payload.turn_id) ?? this.currentTurnId;
    if (!turnId) {
      this.pushMissingTurn(envelope.ordinal, "turn_context");
      return;
    }

    this.currentTurnId = turnId;
    const turn = this.ensureTurn(turnId);
    turn.model = stringValue(envelope.payload.model) ?? turn.model;
    turn.reasoningEffort =
      stringValue(envelope.payload.effort) ?? turn.reasoningEffort;
    turn.cwd = stringValue(envelope.payload.cwd) ?? turn.cwd;

    for (const call of this.modelCalls) {
      if (call.turnId !== turnId) {
        continue;
      }
      call.model ??= turn.model;
      call.reasoningEffort ??= turn.reasoningEffort;
      call.modelProvider ??= turn.modelProvider;
    }
  }

  private parseTimelineItem(
    envelope: RolloutEnvelope,
    eventKey: string,
  ): void {
    const payload = envelope.payload;
    const itemType = stringValue(payload.type) ?? "unknown";

    this.timeline.push({
      eventKey,
      sessionId: this.session!.sessionId,
      turnId: this.currentTurnId,
      ordinal: envelope.ordinal,
      timestamp: envelope.timestamp,
      itemId: stringValue(payload.id) ?? stringValue(payload.call_id),
      itemType,
      role: stringValue(payload.role),
      phase: stringValue(payload.phase),
      toolName: stringValue(payload.name),
      contentUtf8Bytes:
        itemType === "message" ? messageContentUtf8Bytes(payload.content) : undefined,
      contentPersisted: false,
    });
  }

  private parseEventMessage(
    envelope: RolloutEnvelope,
    eventKey: string,
  ): void {
    const eventType = stringValue(envelope.payload.type);
    switch (eventType) {
      case "task_started":
        this.parseTaskStarted(envelope);
        break;
      case "token_count":
        this.parseTokenCount(envelope, eventKey);
        break;
      case "task_complete":
        this.parseTaskComplete(envelope);
        break;
      case "turn_aborted":
        this.parseTurnAborted(envelope);
        break;
      default:
        this.countUnhandledEvent(
          "event_msg:" + safeEventTypeLabel(eventType),
        );
        break;
    }
  }

  private parseTaskStarted(envelope: RolloutEnvelope): void {
    const turnId = stringValue(envelope.payload.turn_id);
    if (!turnId) {
      this.pushMissingTurn(envelope.ordinal, "task_started");
      return;
    }

    this.currentTurnId = turnId;
    const turn = this.ensureTurn(turnId);
    turn.startedAt =
      stringValue(envelope.payload.started_at) ??
      envelope.timestamp ??
      turn.startedAt;
    turn.status = "running";
  }

  private parseTokenCount(
    envelope: RolloutEnvelope,
    eventKey: string,
  ): void {
    if (!this.currentTurnId) {
      this.pushMissingTurn(envelope.ordinal, "token_count");
      return;
    }

    const info = recordValue(envelope.payload.info);
    const cumulativeUsage = parseTokenUsage(info?.total_token_usage);
    const reportedLastUsage = parseTokenUsage(info?.last_token_usage);

    if (!cumulativeUsage) {
      this.diagnostics.push({
        code: "invalid_usage",
        severity: "warning",
        ordinal: envelope.ordinal,
        message: "Skipped token_count without a complete cumulative usage vector.",
      });
      return;
    }

    let callUsage: TokenUsage;
    let usageConfidence: MetricConfidence;
    const turn = this.ensureTurn(this.currentTurnId);

    if (!this.lastCumulativeUsage) {
      if (
        reportedLastUsage &&
        isSameUsage(reportedLastUsage, cumulativeUsage)
      ) {
        callUsage = reportedLastUsage;
        usageConfidence = "codex-reported";
      } else {
        callUsage = cumulativeUsage;
        usageConfidence = "derived";
        if (reportedLastUsage) {
          this.diagnostics.push({
            code: "reported_usage_mismatch",
            severity: "warning",
            ordinal: envelope.ordinal,
            message:
              "Used the first cumulative token vector because reported last usage did not match it.",
          });
        }
      }
    } else if (isSameUsage(this.lastCumulativeUsage, cumulativeUsage)) {
      this.ignoredDuplicateUsageSnapshots += 1;
      return;
    } else if (
      isMonotonicUsage(this.lastCumulativeUsage, cumulativeUsage)
    ) {
      const delta = subtractUsage(cumulativeUsage, this.lastCumulativeUsage);
      if (
        reportedLastUsage &&
        isSameUsage(reportedLastUsage, delta)
      ) {
        callUsage = reportedLastUsage;
        usageConfidence = "codex-reported";
      } else {
        callUsage = delta;
        usageConfidence = "derived";
      }
    } else {
      if (
        turn.modelCallCount === 0 &&
        !isZeroUsage(cumulativeUsage)
      ) {
        if (
          reportedLastUsage &&
          isSameUsage(reportedLastUsage, cumulativeUsage)
        ) {
          callUsage = reportedLastUsage;
          usageConfidence = "codex-reported";
        } else {
          callUsage = cumulativeUsage;
          usageConfidence = "derived";
          if (reportedLastUsage) {
            this.diagnostics.push({
              code: "reported_usage_mismatch",
              severity: "warning",
              ordinal: envelope.ordinal,
              message:
                "Used the reset cumulative token vector because reported last usage did not match it.",
            });
          }
        }
        this.diagnostics.push({
          code: "non_monotonic_usage",
          severity: "info",
          ordinal: envelope.ordinal,
          message:
            "Accepted the first snapshot after a new-turn token counter reset.",
        });
      } else {
        this.diagnostics.push({
          code: "non_monotonic_usage",
          severity: "warning",
          ordinal: envelope.ordinal,
          message:
            "Quarantined a token snapshot whose cumulative vector moved backwards.",
        });
        return;
      }
    }

    if (!isValidCallUsage(callUsage)) {
      this.diagnostics.push({
        code: "usage_call_out_of_range",
        severity: "warning",
        ordinal: envelope.ordinal,
        message:
          "Quarantined a token vector that was inconsistent or exceeded the per-call budget.",
      });
      return;
    }

    this.lastCumulativeUsage = cumulativeUsage;
    if (isZeroUsage(callUsage)) {
      this.ignoredDuplicateUsageSnapshots += 1;
      return;
    }

    let nextTurnUsage: TokenUsage;
    try {
      nextTurnUsage = addUsage(turn.usage, callUsage);
    } catch {
      this.diagnostics.push({
        code: "usage_overflow",
        severity: "warning",
        ordinal: envelope.ordinal,
        message: "Quarantined token usage that exceeded the safe integer range.",
      });
      return;
    }

    const modelCall: ModelCallRecord = {
      eventKey,
      sessionId: this.session!.sessionId,
      turnId: this.currentTurnId,
      ordinal: envelope.ordinal,
      timestamp: envelope.timestamp,
      model: turn.model,
      reasoningEffort: turn.reasoningEffort,
      modelProvider: turn.modelProvider,
      usage: callUsage,
      cumulativeUsage,
      usageConfidence,
    };

    this.modelCalls.push(modelCall);
    turn.usage = nextTurnUsage;
    turn.usageConfidence =
      turn.usageConfidence === "unavailable"
        ? usageConfidence
        : turn.usageConfidence === usageConfidence
          ? usageConfidence
          : "derived";
    turn.modelCallCount += 1;
  }

  private parseTaskComplete(envelope: RolloutEnvelope): void {
    const turnId =
      stringValue(envelope.payload.turn_id) ?? this.currentTurnId;
    if (!turnId) {
      this.pushMissingTurn(envelope.ordinal, "task_complete");
      return;
    }

    const turn = this.ensureTurn(turnId);
    turn.startedAt =
      stringValue(envelope.payload.started_at) ?? turn.startedAt;
    turn.completedAt =
      stringValue(envelope.payload.completed_at) ??
      envelope.timestamp ??
      turn.completedAt;
    turn.durationMs =
      numberValue(envelope.payload.duration_ms) ?? turn.durationMs;
    turn.timeToFirstTokenMs =
      numberValue(envelope.payload.time_to_first_token_ms) ??
      turn.timeToFirstTokenMs;
    turn.status = envelope.payload.error ? "failed" : "completed";
    turn.timingConfidence = "codex-reported";
    this.currentTurnId = turnId;
  }

  private parseTurnAborted(envelope: RolloutEnvelope): void {
    const turnId =
      stringValue(envelope.payload.turn_id) ?? this.currentTurnId;
    if (!turnId) {
      this.pushMissingTurn(envelope.ordinal, "turn_aborted");
      return;
    }

    const turn = this.ensureTurn(turnId);
    turn.startedAt =
      stringValue(envelope.payload.started_at) ?? turn.startedAt;
    turn.completedAt =
      stringValue(envelope.payload.completed_at) ??
      envelope.timestamp ??
      turn.completedAt;
    turn.durationMs =
      numberValue(envelope.payload.duration_ms) ?? turn.durationMs;
    turn.status = "aborted";
    turn.timingConfidence = "codex-reported";
    this.currentTurnId = turnId;
  }

  private ensureTurn(turnId: string): TurnRecord {
    const existing = this.turns.get(turnId);
    if (existing) {
      return existing;
    }

    const turn: TurnRecord = {
      turnId,
      sessionId: this.session!.sessionId,
      modelProvider: this.session!.modelProvider,
      cwd: this.session!.cwd,
      status: "unknown",
      timingConfidence: "unavailable",
      usage: { ...ZERO_USAGE },
      usageConfidence: "unavailable",
      modelCallCount: 0,
    };
    this.turns.set(turnId, turn);
    return turn;
  }

  private createEventKey(
    envelope: RolloutEnvelope,
    sourcePosition?: number | string,
  ): string {
    const sessionId =
      this.session?.sessionId ??
      stringValue(envelope.payload.session_id) ??
      stringValue(envelope.payload.id) ??
      "unknown-session";
    const position = eventPosition(envelope, sourcePosition);
    const subtype = stringValue(envelope.payload.type) ?? "none";
    return [sessionId, position, envelope.type, subtype].join(":");
  }

  private countUnhandledEvent(eventType: string): void {
    this.unhandledEventCounts.set(
      eventType,
      (this.unhandledEventCounts.get(eventType) ?? 0) + 1,
    );
  }

  private pushMissingTurn(ordinal: number | undefined, eventType: string): void {
    this.diagnostics.push({
      code: "missing_turn",
      severity: "warning",
      ordinal,
      message: "Skipped " + eventType + " because no active turn was known.",
    });
  }
}

function parseEnvelope(value: unknown): RolloutEnvelope | undefined {
  if (!isRecord(value) || typeof value.type !== "string") {
    return undefined;
  }

  const payload = recordValue(value.payload);
  if (!payload) {
    return undefined;
  }

  return {
    ordinal: numberValue(value.ordinal),
    timestamp: stringValue(value.timestamp),
    type: value.type,
    payload,
  };
}

function eventPosition(
  envelope: RolloutEnvelope,
  sourcePosition?: number | string,
): string {
  if (envelope.ordinal !== undefined) {
    return "ordinal-" + envelope.ordinal;
  }
  if (sourcePosition !== undefined) {
    return "offset-" + sourcePosition;
  }

  const itemIdentity =
    stringValue(envelope.payload.id) ??
    stringValue(envelope.payload.call_id) ??
    stringValue(envelope.payload.turn_id);
  if (itemIdentity) {
    return "id-" + itemIdentity;
  }

  if (
    envelope.type === "event_msg" &&
    stringValue(envelope.payload.type) === "token_count"
  ) {
    const info = recordValue(envelope.payload.info);
    const usage = parseTokenUsage(info?.total_token_usage);
    if (usage) {
      return (
        "usage-" +
        [
          usage.inputTokens,
          usage.cachedInputTokens,
          usage.cacheWriteInputTokens,
          usage.outputTokens,
          usage.reasoningOutputTokens,
          usage.totalTokens,
        ].join(",")
      );
    }
  }

  return "timestamp-" + (envelope.timestamp ?? "unknown");
}

function cloneTurn(turn: TurnRecord): TurnRecord {
  return {
    ...turn,
    usage: { ...turn.usage },
  };
}

function messageContentUtf8Bytes(value: unknown): number | undefined {
  if (!Array.isArray(value)) {
    return undefined;
  }

  let sawText = false;
  let totalBytes = 0;
  for (const part of value) {
    if (!isRecord(part)) {
      continue;
    }
    const text = stringValue(part.text);
    if (text !== undefined) {
      sawText = true;
      totalBytes += new TextEncoder().encode(text).byteLength;
    }
  }
  return sawText ? totalBytes : undefined;
}

function sourceKind(value: unknown): string | undefined {
  if (typeof value === "string") {
    return value;
  }
  if (isRecord(value)) {
    return stringValue(value.type) ?? stringValue(value.kind);
  }
  return undefined;
}

function stringValue(value: unknown): string | undefined {
  return typeof value === "string" ? value : undefined;
}

function numberValue(value: unknown): number | undefined {
  return typeof value === "number" && Number.isFinite(value)
    ? value
    : undefined;
}

function recordValue(value: unknown): JsonObject | undefined {
  return isRecord(value) ? value : undefined;
}

function isRecord(value: unknown): value is JsonObject {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function safeEventTypeLabel(value: string | undefined): string {
  if (value && /^[A-Za-z0-9_.-]{1,64}$/.test(value)) {
    return value;
  }
  return "other";
}
