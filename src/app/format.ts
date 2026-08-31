import type { MetricConfidence, MetricSource, MetricValue } from "../shared/contracts.ts";
import { getCurrentLocale, t, type Locale } from "./i18n-core.ts";

function numberFormatter(locale: Locale): Intl.NumberFormat {
  return new Intl.NumberFormat(locale, { maximumFractionDigits: 0 });
}

function compactNumberFormatter(locale: Locale): Intl.NumberFormat {
  return new Intl.NumberFormat(locale, { notation: "compact", maximumFractionDigits: 1 });
}

function dateFormatter(locale: Locale): Intl.DateTimeFormat {
  return new Intl.DateTimeFormat(locale, { month: "2-digit", day: "2-digit", hour: "2-digit", minute: "2-digit", hour12: false });
}

function buildTimeFormatter(locale: Locale): Intl.DateTimeFormat {
  return new Intl.DateTimeFormat(locale, { year: "numeric", month: "2-digit", day: "2-digit", hour: "2-digit", minute: "2-digit", hour12: false });
}

export function formatCount(value: number | null | undefined, compact = false): string {
  if (value === null || value === undefined) return "unavailable";
  const locale = getCurrentLocale();
  return (compact ? compactNumberFormatter(locale) : numberFormatter(locale)).format(value);
}

export function formatUsd(value: number | null | undefined, maximumFractionDigits = 4): string {
  if (value === null || value === undefined) return "unavailable";
  return new Intl.NumberFormat(getCurrentLocale(), {
    style: "currency",
    currency: "USD",
    maximumFractionDigits,
    minimumFractionDigits: 2,
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
  return Number.isNaN(parsed.valueOf()) ? "unavailable" : dateFormatter(getCurrentLocale()).format(parsed);
}

export function formatBuildTime(value: string | null | undefined): string {
  if (!value) return "unavailable";
  const parsed = new Date(value);
  return Number.isNaN(parsed.valueOf()) ? "unavailable" : buildTimeFormatter(getCurrentLocale()).format(parsed);
}

export function formatMetric<T>(metric: MetricValue<T>, render: (value: T) => string = String): string {
  return metric.value === null ? "unavailable" : render(metric.value);
}

export function confidenceLabel(confidence: MetricConfidence): string {
  return t({
    high: "高可信",
    medium: "中可信",
    low: "低可信",
    unavailable: "不可用",
  }[confidence] ?? "不可用");
}

export function metricSourceLabel(source: MetricSource): string {
  return t({
    rollout: "Codex 本地记录",
    derived: "由已观测数据计算",
    configured: "价格目录规则",
    server: "服务端或 OTel 请求元数据",
    manual: "手动配置",
    unavailable: "此来源未提供",
  }[source] ?? "此来源未提供");
}

export function metricProvenanceLabel(metric: Pick<MetricValue<unknown>, "source" | "confidence">): string {
  const confidence = metric.confidence === "unavailable" ? t("无法评估") : confidenceLabel(metric.confidence);
  return `${metricSourceLabel(metric.source)} · ${confidence}`;
}
