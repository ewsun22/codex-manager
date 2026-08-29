import type { AppUpdateStatus } from "../shared/contracts.ts";

export const DEFAULT_UPDATE_CHECK_INTERVAL_HOURS = 12;
export const MIN_UPDATE_CHECK_INTERVAL_HOURS = 1;
export const MAX_UPDATE_CHECK_INTERVAL_HOURS = 168;

export interface UpdateCheckScheduleInput {
  now: number;
  status: Pick<AppUpdateStatus, "checkedAt"> | null;
  intervalHours: number | null | undefined;
  lastAttemptAt: string | null;
}

export interface UpdateCheckSchedule {
  due: boolean;
  delayMs: number;
}

export function markUpdateStatusUninstallable(status: AppUpdateStatus | null): AppUpdateStatus | null {
  return status ? { ...status, installable: false } : null;
}

export function normalizeUpdateCheckIntervalHours(value: number | null | undefined): number {
  if (!Number.isFinite(value)) return DEFAULT_UPDATE_CHECK_INTERVAL_HOURS;
  return Math.min(MAX_UPDATE_CHECK_INTERVAL_HOURS, Math.max(MIN_UPDATE_CHECK_INTERVAL_HOURS, Math.floor(value!)));
}

function timestampMs(value: string | null): number | null {
  if (!value) return null;
  const parsed = Date.parse(value);
  return Number.isFinite(parsed) ? parsed : null;
}

export function getUpdateCheckSchedule({ now, status, intervalHours, lastAttemptAt }: UpdateCheckScheduleInput): UpdateCheckSchedule {
  const lastEffectiveCheckAt = Math.max(
    timestampMs(status?.checkedAt ?? null) ?? Number.NEGATIVE_INFINITY,
    timestampMs(lastAttemptAt) ?? Number.NEGATIVE_INFINITY,
  );
  if (Number.isFinite(lastEffectiveCheckAt)) {
    const delayMs = Math.max(0, lastEffectiveCheckAt + normalizeUpdateCheckIntervalHours(intervalHours) * 60 * 60 * 1_000 - now);
    return { due: delayMs === 0, delayMs };
  }

  return { due: true, delayMs: 0 };
}
