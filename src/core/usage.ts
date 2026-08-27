import type { TokenUsage } from "./types.ts";

export const ZERO_USAGE: TokenUsage = Object.freeze({
  inputTokens: 0,
  cachedInputTokens: 0,
  cacheWriteInputTokens: 0,
  outputTokens: 0,
  reasoningOutputTokens: 0,
  totalTokens: 0,
});

export const MAX_TOKEN_FIELD_VALUE = 1_000_000_000_000;
export const MAX_CALL_TOKEN_FIELD_VALUE = 10_000_000;

const SOURCE_KEYS = {
  inputTokens: "input_tokens",
  cachedInputTokens: "cached_input_tokens",
  cacheWriteInputTokens: "cache_write_input_tokens",
  outputTokens: "output_tokens",
  reasoningOutputTokens: "reasoning_output_tokens",
  totalTokens: "total_tokens",
} as const;

export function parseTokenUsage(value: unknown): TokenUsage | undefined {
  if (!isRecord(value)) {
    return undefined;
  }

  const usage = {} as TokenUsage;
  for (const [targetKey, sourceKey] of Object.entries(SOURCE_KEYS)) {
    const numberValue = value[sourceKey];
    if (
      typeof numberValue !== "number" ||
      !Number.isSafeInteger(numberValue) ||
      numberValue < 0 ||
      numberValue > MAX_TOKEN_FIELD_VALUE
    ) {
      return undefined;
    }
    usage[targetKey as keyof TokenUsage] = numberValue;
  }
  return isConsistentUsage(usage) ? usage : undefined;
}

export function isConsistentUsage(usage: TokenUsage): boolean {
  return (
    usage.cachedInputTokens + usage.cacheWriteInputTokens <=
      usage.inputTokens &&
    usage.reasoningOutputTokens <= usage.outputTokens &&
    usage.inputTokens + usage.outputTokens === usage.totalTokens
  );
}

export function isValidCallUsage(usage: TokenUsage): boolean {
  return (
    isConsistentUsage(usage) &&
    everyUsageKey((key) => usage[key] <= MAX_CALL_TOKEN_FIELD_VALUE)
  );
}

export function addUsage(left: TokenUsage, right: TokenUsage): TokenUsage {
  return mapUsage((key) => checkedAdd(left[key], right[key]));
}

export function subtractUsage(
  current: TokenUsage,
  previous: TokenUsage,
): TokenUsage {
  return mapUsage((key) => current[key] - previous[key]);
}

export function isMonotonicUsage(
  previous: TokenUsage,
  current: TokenUsage,
): boolean {
  return everyUsageKey((key) => current[key] >= previous[key]);
}

export function isSameUsage(left: TokenUsage, right: TokenUsage): boolean {
  return everyUsageKey((key) => left[key] === right[key]);
}

export function isZeroUsage(usage: TokenUsage): boolean {
  return everyUsageKey((key) => usage[key] === 0);
}

function mapUsage(
  mapper: (key: keyof TokenUsage) => number,
): TokenUsage {
  return {
    inputTokens: mapper("inputTokens"),
    cachedInputTokens: mapper("cachedInputTokens"),
    cacheWriteInputTokens: mapper("cacheWriteInputTokens"),
    outputTokens: mapper("outputTokens"),
    reasoningOutputTokens: mapper("reasoningOutputTokens"),
    totalTokens: mapper("totalTokens"),
  };
}

function everyUsageKey(
  predicate: (key: keyof TokenUsage) => boolean,
): boolean {
  return (
    predicate("inputTokens") &&
    predicate("cachedInputTokens") &&
    predicate("cacheWriteInputTokens") &&
    predicate("outputTokens") &&
    predicate("reasoningOutputTokens") &&
    predicate("totalTokens")
  );
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function checkedAdd(left: number, right: number): number {
  const result = left + right;
  if (
    !Number.isSafeInteger(result) ||
    result < 0 ||
    result > MAX_TOKEN_FIELD_VALUE
  ) {
    throw new RangeError("token usage exceeds the supported field range");
  }
  return result;
}
