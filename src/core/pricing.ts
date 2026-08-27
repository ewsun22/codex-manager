import type { TokenUsage } from "./types.ts";

export interface PricingRule {
  catalogVersion: string;
  provider: string;
  model: string;
  currency: "USD";
  effectiveAt: string;
  ratesMicrosPerMillionTokens: {
    input: number;
    cachedInput: number;
    cacheWriteInput: number;
    output: number;
  };
  longContext?: {
    inputTokenThresholdExclusive: number;
    inputMultiplier: number;
    outputMultiplier: number;
  };
}

export interface CostEstimate {
  catalogVersion: string;
  currency: "USD";
  confidence: "derived";
  plainInputTokens: number;
  inputCostMicros: number;
  cachedInputCostMicros: number;
  cacheWriteInputCostMicros: number;
  outputCostMicros: number;
  totalCostMicros: number;
  longContextPricingApplied: boolean;
}

export function estimateApiEquivalentCost(
  usage: TokenUsage,
  rule: PricingRule,
): CostEstimate {
  const plainInputTokens =
    usage.inputTokens -
    usage.cachedInputTokens -
    usage.cacheWriteInputTokens;

  if (plainInputTokens < 0) {
    throw new RangeError(
      "cached and cache-write input tokens cannot exceed input tokens",
    );
  }

  const longContextPricingApplied = Boolean(
    rule.longContext &&
      usage.inputTokens > rule.longContext.inputTokenThresholdExclusive,
  );
  const inputMultiplier = longContextPricingApplied
    ? validatedMultiplier(rule.longContext!.inputMultiplier)
    : 1;
  const outputMultiplier = longContextPricingApplied
    ? validatedMultiplier(rule.longContext!.outputMultiplier)
    : 1;

  const inputCostMicros = calculateMicros(
    plainInputTokens,
    rule.ratesMicrosPerMillionTokens.input * inputMultiplier,
  );
  const cachedInputCostMicros = calculateMicros(
    usage.cachedInputTokens,
    rule.ratesMicrosPerMillionTokens.cachedInput * inputMultiplier,
  );
  const cacheWriteInputCostMicros = calculateMicros(
    usage.cacheWriteInputTokens,
    rule.ratesMicrosPerMillionTokens.cacheWriteInput * inputMultiplier,
  );
  const outputCostMicros = calculateMicros(
    usage.outputTokens,
    rule.ratesMicrosPerMillionTokens.output * outputMultiplier,
  );

  return {
    catalogVersion: rule.catalogVersion,
    currency: rule.currency,
    confidence: "derived",
    plainInputTokens,
    inputCostMicros,
    cachedInputCostMicros,
    cacheWriteInputCostMicros,
    outputCostMicros,
    longContextPricingApplied,
    totalCostMicros:
      inputCostMicros +
      cachedInputCostMicros +
      cacheWriteInputCostMicros +
      outputCostMicros,
  };
}

function validatedMultiplier(multiplier: number): number {
  if (!Number.isFinite(multiplier) || multiplier <= 0) {
    throw new RangeError("pricing multipliers must be finite and positive");
  }
  return multiplier;
}

function calculateMicros(
  tokens: number,
  rateMicrosPerMillionTokens: number,
): number {
  if (!Number.isFinite(rateMicrosPerMillionTokens) || rateMicrosPerMillionTokens < 0) {
    throw new RangeError("pricing rates must be finite and non-negative");
  }
  return Math.round((tokens * rateMicrosPerMillionTokens) / 1_000_000);
}
