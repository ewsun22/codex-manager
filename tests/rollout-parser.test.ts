import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { test } from "node:test";
import { RolloutNormalizer } from "../src/core/rollout-parser.ts";
import {
  MAX_CALL_TOKEN_FIELD_VALUE,
  MAX_TOKEN_FIELD_VALUE,
  addUsage,
  parseTokenUsage,
} from "../src/core/usage.ts";

test("normalizes a turn without persisting message content", async () => {
  const normalizer = await parseFixture("rollout-basic.jsonl");
  const snapshot = normalizer.snapshot();

  assert.equal(snapshot.session?.cliVersion, "0.fixture");
  assert.equal(snapshot.turns.length, 1);
  assert.equal(snapshot.modelCalls.length, 2);
  assert.equal(snapshot.timeline.length, 2);
  assert.equal(snapshot.ignoredDuplicateUsageSnapshots, 1);

  const turn = snapshot.turns[0];
  assert.equal(turn.model, "gpt-fixture");
  assert.equal(turn.reasoningEffort, "high");
  assert.equal(turn.status, "completed");
  assert.equal(turn.durationMs, 2500);
  assert.equal(turn.timeToFirstTokenMs, 400);
  assert.equal(turn.modelCallCount, 2);
  assert.deepEqual(turn.usage, {
    inputTokens: 150,
    cachedInputTokens: 70,
    cacheWriteInputTokens: 10,
    outputTokens: 30,
    reasoningOutputTokens: 7,
    totalTokens: 180,
  });

  assert.equal(snapshot.timeline[0].contentUtf8Bytes, 5);
  assert.equal(snapshot.timeline[0].contentPersisted, false);
  assert.equal("content" in snapshot.timeline[0], false);
});

test("deduplicates the same event identity", async () => {
  const contents = await readFixture("rollout-basic.jsonl");
  const lines = contents.trimEnd().split("\n");
  const normalizer = new RolloutNormalizer();

  for (const line of lines) {
    normalizer.parseLine(line);
  }
  normalizer.parseLine(lines[3]);

  const snapshot = normalizer.snapshot();
  assert.equal(snapshot.timeline.length, 2);
  assert.equal(snapshot.ignoredDuplicateEvents, 1);
});

test("does not double-count a copied rollout", async () => {
  const contents = await readFixture("rollout-basic.jsonl");
  const lines = contents.trimEnd().split("\n");
  const normalizer = new RolloutNormalizer();

  for (const line of [...lines, ...lines]) {
    normalizer.parseLine(line);
  }

  const snapshot = normalizer.snapshot();
  assert.equal(snapshot.modelCalls.length, 2);
  assert.equal(snapshot.turns[0].usage.totalTokens, 180);
  assert.equal(snapshot.timeline.length, 2);
  assert.equal(snapshot.ignoredDuplicateEvents, lines.length);
});

test("keeps distinct same-timestamp events when ordinal is absent", () => {
  const normalizer = new RolloutNormalizer();
  const timestamp = "2026-08-27T00:00:00.000Z";

  normalizer.parseLine(
    event(timestamp, "session_meta", {
      id: "session-no-ordinal",
      session_id: "session-no-ordinal",
    }),
  );
  normalizer.parseLine(
    event(timestamp, "event_msg", {
      type: "task_started",
      turn_id: "turn-no-ordinal",
    }),
  );
  normalizer.parseLine(
    event(timestamp, "turn_context", {
      turn_id: "turn-no-ordinal",
      model: "gpt-fixture",
      effort: "high",
    }),
  );
  normalizer.parseLine(
    event(timestamp, "response_item", {
      type: "message",
      id: "message-one",
      role: "assistant",
      content: [{ type: "output_text", text: "one" }],
    }),
  );
  normalizer.parseLine(
    event(timestamp, "response_item", {
      type: "message",
      id: "message-two",
      role: "assistant",
      content: [{ type: "output_text", text: "two" }],
    }),
  );
  normalizer.parseLine(tokenEvent(timestamp, 11, 11));
  normalizer.parseLine(tokenEvent(timestamp, 22, 11));

  const snapshot = normalizer.snapshot();
  assert.equal(snapshot.timeline.length, 2);
  assert.equal(snapshot.modelCalls.length, 2);
  assert.equal(snapshot.turns[0].usage.totalTokens, 22);
  assert.equal(snapshot.ignoredDuplicateEvents, 0);
});

test("accepts a reported counter reset at a new turn", () => {
  const normalizer = new RolloutNormalizer();
  normalizer.parseLine(
    event("2026-08-27T00:00:00Z", "session_meta", {
      id: "session-reset",
      session_id: "session-reset",
    }),
  );

  startTurn(normalizer, "turn-one", 1);
  normalizer.parseLine(tokenEvent("2026-08-27T00:00:01Z", 110, 110, 2));

  startTurn(normalizer, "turn-two", 3);
  normalizer.parseLine(tokenEvent("2026-08-27T00:00:02Z", 22, 22, 4));
  normalizer.parseLine(tokenEvent("2026-08-27T00:00:03Z", 33, 11, 5));

  const snapshot = normalizer.snapshot();
  const secondTurn = snapshot.turns.find((turn) => turn.turnId === "turn-two");
  assert.equal(secondTurn?.modelCallCount, 2);
  assert.equal(secondTurn?.usage.totalTokens, 33);
  assert.equal(
    snapshot.diagnostics.filter(
      (diagnostic) =>
        diagnostic.code === "non_monotonic_usage" &&
        diagnostic.severity === "info",
    ).length,
    1,
  );
});

test("derives a new-turn counter reset when last usage is absent", () => {
  const normalizer = new RolloutNormalizer();
  normalizer.parseLine(
    event("2026-08-27T00:00:00Z", "session_meta", {
      id: "session-reset-derived",
      session_id: "session-reset-derived",
    }),
  );

  startTurn(normalizer, "turn-one", 1);
  normalizer.parseLine(tokenEvent("2026-08-27T00:00:01Z", 100, 100, 2));
  startTurn(normalizer, "turn-two", 3);
  normalizer.parseLine(
    event(
      "2026-08-27T00:00:02Z",
      "event_msg",
      {
        type: "token_count",
        info: {
          total_token_usage: usage(20),
        },
      },
      4,
    ),
  );

  const snapshot = normalizer.snapshot();
  const secondTurn = snapshot.turns.find((turn) => turn.turnId === "turn-two");
  assert.equal(secondTurn?.modelCallCount, 1);
  assert.equal(secondTurn?.usage.totalTokens, 20);
  assert.equal(secondTurn?.usageConfidence, "derived");
});

test("backfills model metadata when turn context arrives after usage", () => {
  const normalizer = new RolloutNormalizer();
  normalizer.parseLine(
    event("2026-08-27T00:00:00Z", "session_meta", {
      id: "session-late-context",
      session_id: "session-late-context",
      model_provider: "openai",
    }),
  );
  normalizer.parseLine(
    event("2026-08-27T00:00:01Z", "event_msg", {
      type: "task_started",
      turn_id: "turn-late-context",
    }, 1),
  );
  normalizer.parseLine(tokenEvent("2026-08-27T00:00:02Z", 11, 11, 2));
  normalizer.parseLine(
    event("2026-08-27T00:00:03Z", "turn_context", {
      turn_id: "turn-late-context",
      model: "gpt-late",
      effort: "xhigh",
    }, 3),
  );

  const call = normalizer.snapshot().modelCalls[0];
  assert.equal(call.model, "gpt-late");
  assert.equal(call.reasoningEffort, "xhigh");
  assert.equal(call.modelProvider, "openai");
});

test("records an aborted turn without persisting its reason", () => {
  const normalizer = new RolloutNormalizer();
  normalizer.parseLine(
    event("2026-08-27T00:00:00Z", "session_meta", {
      id: "session-aborted",
      session_id: "session-aborted",
    }),
  );
  startTurn(normalizer, "turn-aborted", 1);
  normalizer.parseLine(
    event(
      "2026-08-27T00:00:02Z",
      "event_msg",
      {
        type: "turn_aborted",
        turn_id: "turn-aborted",
        started_at: "2026-08-27T00:00:00Z",
        completed_at: "2026-08-27T00:00:02Z",
        duration_ms: 2000,
        reason: "fixture-sensitive-reason",
      },
      2,
    ),
  );

  const snapshot = normalizer.snapshot();
  assert.equal(snapshot.turns[0].status, "aborted");
  assert.equal(snapshot.turns[0].durationMs, 2000);
  assert.equal(
    JSON.stringify(snapshot).includes("fixture-sensitive-reason"),
    false,
  );
  assert.equal(snapshot.unhandledEventCounts["event_msg:turn_aborted"], undefined);
});

test("quarantines non-monotonic cumulative token snapshots", async () => {
  const normalizer = await parseFixture("rollout-non-monotonic.jsonl");
  const snapshot = normalizer.snapshot();

  assert.equal(snapshot.modelCalls.length, 1);
  assert.equal(
    snapshot.diagnostics.filter(
      (diagnostic) => diagnostic.code === "non_monotonic_usage",
    ).length,
    1,
  );
});

test("does not let an in-turn regression contaminate the usage baseline", () => {
  const normalizer = new RolloutNormalizer();
  normalizer.parseLine(
    event("2026-08-27T00:00:00Z", "session_meta", {
      id: "session-regression",
      session_id: "session-regression",
    }),
  );
  startTurn(normalizer, "turn-regression", 1);
  normalizer.parseLine(tokenEvent("2026-08-27T00:00:01Z", 100, 100, 2));
  normalizer.parseLine(tokenEvent("2026-08-27T00:00:02Z", 20, 20, 3));
  normalizer.parseLine(tokenEvent("2026-08-27T00:00:03Z", 110, 10, 4));

  const snapshot = normalizer.snapshot();
  assert.equal(snapshot.modelCalls.length, 2);
  assert.equal(snapshot.turns[0].usage.totalTokens, 110);
  assert.equal(snapshot.modelCalls[1].usage.totalTokens, 10);
});

test("isolates malformed JSON lines", () => {
  const normalizer = new RolloutNormalizer();
  normalizer.parseLine("{not-json");
  const snapshot = normalizer.snapshot();

  assert.equal(snapshot.diagnostics[0].code, "invalid_json");
});

test("redacts unsafe unknown event type labels", () => {
  const normalizer = new RolloutNormalizer();
  normalizer.parseLine(
    event("2026-08-27T00:00:00Z", "session_meta", {
      id: "session-unknown",
      session_id: "session-unknown",
    }),
  );
  normalizer.parseLine(
    event("2026-08-27T00:00:01Z", "event_msg", {
      type: "Bearer secret-should-not-appear",
    }, 1),
  );

  const snapshotJson = JSON.stringify(normalizer.snapshot());
  assert.equal(snapshotJson.includes("secret-should-not-appear"), false);
  assert.equal(
    normalizer.snapshot().unhandledEventCounts["event_msg:other"],
    1,
  );
});

test("rejects unsafe token integers and checked-add overflow", () => {
  assert.equal(parseTokenUsage(usage(MAX_TOKEN_FIELD_VALUE + 1)), undefined);
  assert.throws(
    () =>
      addUsage(
        {
          inputTokens: MAX_TOKEN_FIELD_VALUE,
          cachedInputTokens: 0,
          cacheWriteInputTokens: 0,
          outputTokens: 0,
          reasoningOutputTokens: 0,
          totalTokens: MAX_TOKEN_FIELD_VALUE,
        },
        {
          inputTokens: 1,
          cachedInputTokens: 0,
          cacheWriteInputTokens: 0,
          outputTokens: 0,
          reasoningOutputTokens: 0,
          totalTokens: 1,
        },
      ),
    RangeError,
  );
});

test("does not let first reported usage understate cumulative usage", () => {
  const normalizer = new RolloutNormalizer();
  normalizer.parseLine(
    event("2026-08-27T00:00:00Z", "session_meta", {
      id: "session-first-mismatch",
      session_id: "session-first-mismatch",
    }),
  );
  startTurn(normalizer, "turn-first-mismatch", 1);
  normalizer.parseLine(tokenEvent("2026-08-27T00:00:01Z", 100, 1, 2));

  const snapshot = normalizer.snapshot();
  assert.equal(snapshot.modelCalls[0].usage.totalTokens, 100);
  assert.equal(snapshot.modelCalls[0].usageConfidence, "derived");
  assert.equal(
    snapshot.diagnostics.some(
      (diagnostic) => diagnostic.code === "reported_usage_mismatch",
    ),
    true,
  );
});

test("quarantines an implausibly large single model call", () => {
  const normalizer = new RolloutNormalizer();
  normalizer.parseLine(
    event("2026-08-27T00:00:00Z", "session_meta", {
      id: "session-large-call",
      session_id: "session-large-call",
    }),
  );
  startTurn(normalizer, "turn-large-call", 1);
  const excessive = MAX_CALL_TOKEN_FIELD_VALUE + 1;
  normalizer.parseLine(
    tokenEvent("2026-08-27T00:00:01Z", excessive, excessive, 2),
  );

  const snapshot = normalizer.snapshot();
  assert.equal(snapshot.modelCalls.length, 0);
  assert.equal(
    snapshot.diagnostics.some(
      (diagnostic) => diagnostic.code === "usage_call_out_of_range",
    ),
    true,
  );
});

async function parseFixture(fileName: string): Promise<RolloutNormalizer> {
  const contents = await readFixture(fileName);
  const normalizer = new RolloutNormalizer();
  for (const line of contents.trimEnd().split("\n")) {
    normalizer.parseLine(line);
  }
  return normalizer;
}

async function readFixture(fileName: string): Promise<string> {
  return readFile(new URL("./fixtures/" + fileName, import.meta.url), "utf8");
}

function startTurn(
  normalizer: RolloutNormalizer,
  turnId: string,
  ordinal: number,
): void {
  normalizer.parseLine(
    event("2026-08-27T00:00:00Z", "event_msg", {
      type: "task_started",
      turn_id: turnId,
    }, ordinal),
  );
}

function tokenEvent(
  timestamp: string,
  cumulativeTotal: number,
  lastTotal: number,
  ordinal?: number,
): string {
  return event(
    timestamp,
    "event_msg",
    {
      type: "token_count",
      info: {
        last_token_usage: usage(lastTotal),
        total_token_usage: usage(cumulativeTotal),
      },
    },
    ordinal,
  );
}

function usage(totalTokens: number): Record<string, number> {
  return {
    input_tokens: totalTokens,
    cached_input_tokens: 0,
    cache_write_input_tokens: 0,
    output_tokens: 0,
    reasoning_output_tokens: 0,
    total_tokens: totalTokens,
  };
}

function event(
  timestamp: string,
  type: string,
  payload: Record<string, unknown>,
  ordinal?: number,
): string {
  return JSON.stringify({
    timestamp,
    type,
    ...(ordinal === undefined ? {} : { ordinal }),
    payload,
  });
}
