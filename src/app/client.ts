import { invokeDemo } from "./demo.ts";

declare global {
  interface Window {
    __TAURI_INTERNALS__?: unknown;
  }
}

export function isDesktopRuntime(): boolean {
  return typeof window !== "undefined" && Boolean(window.__TAURI_INTERNALS__);
}

export async function invokeBackend<T>(
  command: string,
  args: Record<string, unknown> = {},
): Promise<T> {
  if (!isDesktopRuntime()) {
    return invokeDemo<T>(command, args);
  }

  const tauri = await import("@tauri-apps/api/core");
  return tauri.invoke<T>(command, args);
}
