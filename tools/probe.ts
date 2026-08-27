#!/usr/bin/env node

import { resolve } from "node:path";
import { readCompleteJsonlLines } from "../src/core/jsonl-reader.ts";
import { RolloutNormalizer } from "../src/core/rollout-parser.ts";

const fileArgument = process.argv[2];
if (!fileArgument) {
  console.error("Usage: npm run probe -- /path/to/rollout.jsonl");
  process.exitCode = 2;
} else {
  await runProbe(resolve(fileArgument));
}

async function runProbe(filePath: string): Promise<void> {
  const normalizer = new RolloutNormalizer();
  let completeLines = 0;
  let checkpoint = 0;
  let oversizedLines = 0;

  try {
    for await (const entry of readCompleteJsonlLines(filePath, 0, {
      onDiagnostic: (diagnostic) => {
        oversizedLines += 1;
        if (diagnostic.newlineTerminated) {
          checkpoint = diagnostic.nextOffset;
        }
      },
    })) {
      normalizer.parseLine(entry.line, entry.startOffset);
      completeLines += 1;
      checkpoint = entry.nextOffset;
    }
  } catch (error) {
    const errorCode =
      typeof error === "object" &&
      error !== null &&
      "code" in error &&
      typeof error.code === "string"
        ? error.code
        : "UNKNOWN";
    console.error("Probe failed while reading the source (" + errorCode + ").");
    process.exitCode = 1;
    return;
  }

  const snapshot = normalizer.snapshot();
  const output = {
    probeSchema: "phase0-v1",
    source: {
      sourceKind: "rollout-jsonl",
      pathRedacted: true,
      fileNameRedacted: true,
      completeLines,
      checkpoint,
    },
    session: snapshot.session
      ? {
          cliVersion: snapshot.session.cliVersion,
          modelProvider: snapshot.session.modelProvider,
          sourceKind: snapshot.session.sourceKind,
        }
      : null,
    counts: {
      turns: snapshot.turns.length,
      modelCalls: snapshot.modelCalls.length,
      timelineItems: snapshot.timeline.length,
      diagnostics: snapshot.diagnostics.length,
      oversizedLines,
      ignoredDuplicateEvents: snapshot.ignoredDuplicateEvents,
      ignoredDuplicateUsageSnapshots:
        snapshot.ignoredDuplicateUsageSnapshots,
    },
    turns: snapshot.turns.map((turn) => ({
      model: turn.model,
      reasoningEffort: turn.reasoningEffort,
      status: turn.status,
      durationMs: turn.durationMs,
      timeToFirstTokenMs: turn.timeToFirstTokenMs,
      modelCallCount: turn.modelCallCount,
      usage: turn.usage,
      timingConfidence: turn.timingConfidence,
      usageConfidence: turn.usageConfidence,
    })),
    timelineKinds: countBy(
      snapshot.timeline.map((item) =>
        item.role ? item.itemType + ":" + item.role : item.itemType,
      ),
    ),
    diagnosticCodes: countBy(
      snapshot.diagnostics.map((diagnostic) => diagnostic.code),
    ),
    unhandledEventTypes: snapshot.unhandledEventCounts,
    privacy: {
      messageContentPersisted: false,
      identifiersPrinted: false,
      sourcePathPrinted: false,
    },
  };

  process.stdout.write(JSON.stringify(output, null, 2) + "\n");
}

function countBy(values: string[]): Record<string, number> {
  const counts: Record<string, number> = {};
  for (const value of values) {
    counts[value] = (counts[value] ?? 0) + 1;
  }
  return counts;
}
