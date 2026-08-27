import type { MetricConfidence, MetricValue } from "../shared/contracts.ts";

const numberFormatter = new Intl.NumberFormat("zh-CN", { maximumFractionDigits: 0 });
const compactNumberFormatter = new Intl.NumberFormat("zh-CN", {
  notation: "compact",
  maximumFractionDigits: 1,
});
const dateFormatter = new Intl.DateTimeFormat("zh-CN", {
  month: "2-digit",
  day: "2-digit",
  hour: "2-digit",
  minute: "2-digit",
  hour12: false,
});

export function formatCount(value: number | null | undefined, compact = false): string {
  if (value === null || value === undefined) return "unavailable";
  return (compact ? compactNumberFormatter : numberFormatter).format(value);
}

export function formatUsd(value: number | null | undefined): string {
  if (value === null || value === undefined) return "unavailable";
  return new Intl.NumberFormat("zh-CN", {
    style: "currency",
    currency: "USD",
    maximumFractionDigits: 4,
  }).format(value);
}

export function formatMs(value: number | null | undefined): string {
  if (value === null || value === undefined) return "unavailable";
  if (value < 1_000) return `${Math.round(value)} ms`;
  return `${(value / 1_000).toFixed(value < 10_000 ? 2 : 1)} s`;
}

export function formatRatio(value: number | null | undefined): string {
  if (value === null || value === undefined) return "unavailable";
  return `${(value * 100).toFixed(1)}%`;
}

export function formatDate(value: string | null | undefined): string {
  if (!value) return "unavailable";
  const parsed = new Date(value);
  return Number.isNaN(parsed.valueOf()) ? "unavailable" : dateFormatter.format(parsed);
}

export function formatMetric<T>(metric: MetricValue<T>, render: (value: T) => string = String): string {
  return metric.value === null ? "unavailable" : render(metric.value);
}

export function confidenceLabel(confidence: MetricConfidence): string {
  return {
    high: "高可信",
    medium: "中可信",
    low: "低可信",
    unavailable: "不可用",
  }[confidence];
}
