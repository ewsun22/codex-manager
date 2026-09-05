import { safeErrorMessage } from "./operations.ts";

export interface StatusSnapshot<T> { value: T | null; error: string | null; checkedAt: string | null }

/** Shared read state, with one read in flight and protection from pre-mutation responses. */
export class StatusChannel<T> {
  private snapshot: StatusSnapshot<T> = { value: null, error: null, checkedAt: null };
  private listeners = new Set<() => void>();
  private pending: Promise<T> | null = null;
  private revision = 0;
  getSnapshot = (): StatusSnapshot<T> => this.snapshot;
  subscribe = (listener: () => void): (() => void) => { this.listeners.add(listener); return () => this.listeners.delete(listener); };
  invalidate = (): void => { this.revision += 1; };
  private notify(): void { for (const listener of this.listeners) listener(); }
  publish = (value: T): void => {
    this.invalidate();
    this.snapshot = { value, error: null, checkedAt: new Date().toISOString() };
    this.notify();
  };
  refresh(read: () => Promise<T>): Promise<T> {
    if (this.pending) return this.pending;
    const revision = this.revision;
    const request = Promise.resolve().then(read).then((value) => {
      if (revision === this.revision) { this.publish(value); return value; }
      return this.snapshot.value ?? value;
    }, (error: unknown) => {
      if (revision === this.revision) { this.snapshot = { ...this.snapshot, error: safeErrorMessage(error) }; this.notify(); }
      throw error;
    }).finally(() => { if (this.pending === request) this.pending = null; });
    this.pending = request;
    return request;
  }
}

export function pollWhileVisible(options: {
  visible: () => boolean;
  refresh: () => Promise<unknown>;
  subscribeVisibility: (listener: () => void) => () => void;
  schedule: (listener: () => void) => () => void;
}): () => void {
  let disposed = false;
  const tick = () => { if (!disposed && options.visible()) void options.refresh().catch(() => undefined); };
  const cancelTimer = options.schedule(tick);
  const stopVisibility = options.subscribeVisibility(tick);
  return () => { disposed = true; cancelTimer(); stopVisibility(); };
}
