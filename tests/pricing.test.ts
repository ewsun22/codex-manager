import assert from "node:assert/strict";
import { test } from "node:test";
import { estimateApiEquivalentCost } from "../src/core/pricing.ts";

const rule = {
  catalogVersion: "fixture-2026-08-27",
  provider: "fixture",
  model: "gpt-fixture",
  currency: "USD" as const,
  effectiveAt: "2026-08-27",
  ratesMicrosPerMillionTokens: {
    input: 1_000_000,
    cachedInput: 100_000,
    cacheWriteInput: 2_000_000,
    output: 4_000_000,
  },
};

test("prices mutually exclusive input buckets and output once", () => {
  const estimate = estimateApiEquivalentCost(
    {
      inputTokens: 1_000_000,
      cachedInputTokens: 200_000,
      cacheWriteInputTokens: 100_000,
      outputTokens: 100_000,
      reasoningOutputTokens: 40_000,
      totalTokens: 1_100_000,
    },
    rule,
  );

  assert.equal(estimate.plainInputTokens, 700_000);
  assert.equal(estimate.inputCostMicros, 700_000);
  assert.equal(estimate.cachedInputCostMicros, 20_000);
  assert.equal(estimate.cacheWriteInputCostMicros, 200_000);
  assert.equal(estimate.outputCostMicros, 400_000);
  assert.equal(estimate.totalCostMicros, 1_320_000);
});

test("rejects overlapping input buckets", () => {
  assert.throws(
    () =>
      estimateApiEquivalentCost(
        {
          inputTokens: 10,
          cachedInputTokens: 8,
          cacheWriteInputTokens: 4,
          outputTokens: 0,
          reasoningOutputTokens: 0,
          totalTokens: 10,
        },
        rule,
      ),
    RangeError,
  );
});

test("applies long-context multipliers to the full model call", () => {
  const estimate = estimateApiEquivalentCost(
    {
      inputTokens: 300_000,
      cachedInputTokens: 100_000,
      cacheWriteInputTokens: 50_000,
      outputTokens: 20_000,
      reasoningOutputTokens: 10_000,
      totalTokens: 320_000,
    },
    {
      ...rule,
      longContext: {
        inputTokenThresholdExclusive: 272_000,
        inputMultiplier: 2,
        outputMultiplier: 1.5,
      },
    },
  );

  assert.equal(estimate.longContextPricingApplied, true);
  assert.equal(estimate.inputCostMicros, 300_000);
  assert.equal(estimate.cachedInputCostMicros, 20_000);
  assert.equal(estimate.cacheWriteInputCostMicros, 200_000);
  assert.equal(estimate.outputCostMicros, 120_000);
  assert.equal(estimate.totalCostMicros, 640_000);
});

test("does not double-charge reasoning tokens inside the output bucket", () => {
  const usage = {
    inputTokens: 1_000,
    cachedInputTokens: 100,
    cacheWriteInputTokens: 0,
    outputTokens: 500,
    reasoningOutputTokens: 1,
    totalTokens: 1_500,
  };
  const lowReasoning = estimateApiEquivalentCost(usage, rule);
  const highReasoning = estimateApiEquivalentCost(
    { ...usage, reasoningOutputTokens: usage.outputTokens },
    rule,
  );

  assert.equal(lowReasoning.totalCostMicros, highReasoning.totalCostMicros);
});
