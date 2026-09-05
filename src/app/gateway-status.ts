import { useEffect, useSyncExternalStore } from "react";
import { COMMANDS, type CodexGatewayStatus } from "../shared/contracts.ts";
import { invokeBackend } from "./client.ts";
import { pollWhileVisible, StatusChannel } from "./status-channel.ts";

const channel = new StatusChannel<CodexGatewayStatus>();
const refreshStatus = () => channel.refresh(() => invokeBackend<CodexGatewayStatus>(COMMANDS.getCodexGatewayStatus));

export function useGatewayStatus(busy: boolean) {
  const snapshot = useSyncExternalStore(channel.subscribe, channel.getSnapshot, channel.getSnapshot);
  useEffect(() => {
    if (busy) { channel.invalidate(); return; }
    return pollWhileVisible({
      visible: () => document.visibilityState !== "hidden",
      refresh: refreshStatus,
      schedule: (tick) => { const timer = window.setInterval(tick, 5_000); return () => window.clearInterval(timer); },
      subscribeVisibility: (tick) => { document.addEventListener("visibilitychange", tick); return () => document.removeEventListener("visibilitychange", tick); },
    });
  }, [busy]);
  return { status: snapshot.value, statusError: snapshot.error, checkedAt: snapshot.checkedAt, setStatus: channel.publish, refreshStatus };
}
