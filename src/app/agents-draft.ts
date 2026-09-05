import type { AgentsFileSnapshot } from "../shared/contracts.ts";

/** Drafts remain in memory. The saved base stamp is retained for backend CAS. */
export class AgentsDraft {
  snapshot: AgentsFileSnapshot | null = null;
  content = "";
  get dirty(): boolean { return this.snapshot !== null && this.content !== this.snapshot.content; }
  open(snapshot: AgentsFileSnapshot | null): void {
    if (this.dirty && snapshot?.path === this.snapshot?.path) return;
    this.accept(snapshot);
  }
  edit(content: string, saving: boolean): void { if (!saving) this.content = content; }
  accept(snapshot: AgentsFileSnapshot | null): void {
    this.snapshot = snapshot;
    this.content = snapshot?.content ?? "";
  }
  discard(): void { this.content = this.snapshot?.content ?? ""; }
  async canLeave(saving: boolean, confirmDiscard: () => boolean | Promise<boolean>): Promise<boolean> {
    if (saving) return false;
    if (!this.dirty) return true;
    const snapshot = this.snapshot;
    const content = this.content;
    if (!await confirmDiscard()) return false;
    // An asynchronous answer must not discard edits made after the question.
    if (snapshot !== this.snapshot || content !== this.content) return false;
    this.discard();
    return true;
  }
}
