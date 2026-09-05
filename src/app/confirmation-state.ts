export interface ConfirmationRequest {
  title: string;
  message: string;
  confirmLabel: string;
  destructive?: boolean;
}

export interface PendingConfirmation extends ConfirmationRequest { id: number }

/** Each response belongs to exactly one action; concurrent requests are rejected. */
export class ConfirmationState {
  private nextId = 0;
  private current: PendingConfirmation | null = null;
  private resolve: ((confirmed: boolean) => void) | null = null;
  private listeners = new Set<() => void>();
  getSnapshot = (): PendingConfirmation | null => this.current;
  subscribe = (listener: () => void): (() => void) => { this.listeners.add(listener); return () => this.listeners.delete(listener); };
  private notify(): void { for (const listener of this.listeners) listener(); }
  request = (request: ConfirmationRequest): Promise<boolean> => {
    if (this.current) return Promise.resolve(false);
    return new Promise((resolve) => {
      this.resolve = resolve;
      this.current = { ...request, id: ++this.nextId };
      this.notify();
    });
  };
  answer = (id: number, confirmed: boolean): void => {
    if (id !== this.current?.id) return;
    const resolve = this.resolve;
    this.current = null;
    this.resolve = null;
    this.notify();
    resolve?.(confirmed);
  };
  cancel = (): void => { if (this.current) this.answer(this.current.id, false); };
}

/** Do not queue a second navigation, close or write behind an open confirmation. */
export class ConfirmationAction {
  private active = false;
  private mounted = true;
  private generation = 0;
  get pending(): boolean { return this.active; }
  activate = (): void => { this.mounted = true; };
  deactivate = (): void => { this.mounted = false; this.generation += 1; };
  async run(action: () => Promise<boolean>, before?: () => Promise<boolean>): Promise<boolean> {
    if (this.active || !this.mounted) return false;
    const generation = this.generation;
    this.active = true;
    try {
      if (before && !await before()) return false;
      // Approval belongs to the page that requested it, even if unmount happens
      // after the dialog answers and before this promise continuation executes.
      if (!this.mounted || generation !== this.generation) return false;
      return await action();
    }
    finally { this.active = false; }
  }
}
