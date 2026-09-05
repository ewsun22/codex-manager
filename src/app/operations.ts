import { t, tt } from "./i18n-core.ts";

/** Only display textual errors; never stringify arbitrary native payloads. */
export function safeErrorMessage(error: unknown): string {
  const message = error instanceof Error ? error.message : typeof error === "string" ? error : "";
  if (!message.trim()) return t("操作未完成，请刷新后重试。");
  return message
    .replace(/((?:authorization|set-cookie|cookie)["']?\s*[:=]\s*)[^\r\n]+/gi, "$1[redacted]")
    .replace(/https?:\/\/[^\s<>"']+/gi, (value) => {
      try { const url = new URL(value); url.username = ""; url.password = ""; url.search = ""; url.hash = ""; return url.toString(); }
      catch { return "[redacted URL]"; }
    })
    .replace(/\b(Bearer|Basic)\s+[^\s,;]+/gi, "$1 [redacted]")
    .replace(/((?:api[_-]?key|access[_-]?token|refresh[_-]?token|oauth[_-]?code)["']?\s*[:=]\s*)["']?[^\s,;"']+/gi, "$1[redacted]")
    .replace(/\bsk-[A-Za-z0-9_-]+/g, "[redacted]")
    .slice(0, 600);
}

export interface OperationNotice { tone: "success" | "error" | "info"; message: string }

/** A failed read after a committed write must never be reported as a failed write. */
export async function refreshAfterCommit(message: string, refresh: () => Promise<unknown>, tone: OperationNotice["tone"] = "success"): Promise<OperationNotice> {
  try {
    await refresh();
    return { tone, message };
  } catch (error) {
    return { tone: "info", message: tt`${message} 状态刷新失败：${safeErrorMessage(error)}` };
  }
}

export async function commitAndRefresh<T>(write: () => Promise<T>, publish: (value: T) => void, message: string, refresh: () => Promise<unknown>, tone: OperationNotice["tone"] = "success"): Promise<OperationNotice> {
  const value = await write();
  publish(value);
  return refreshAfterCommit(message, refresh, tone);
}

/** Only the last request may publish data or errors to the current selection. */
export class LatestRequest {
  private revision = 0;
  invalidate(): void { this.revision += 1; }
  async run<T>(read: () => Promise<T>, commit: (value: T) => void): Promise<{ status: "applied" | "ignored" } | { status: "failed"; error: unknown }> {
    const revision = ++this.revision;
    try {
      const value = await read();
      if (revision !== this.revision) return { status: "ignored" };
      commit(value);
      return { status: "applied" };
    } catch (error) {
      return revision === this.revision ? { status: "failed", error } : { status: "ignored" };
    }
  }
}
