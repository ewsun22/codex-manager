import type { AssetRecord } from "./release-state.mjs";

export class GitHubApi {
  constructor(repository: string, token?: string);
  downloadPublic(url: string): Promise<Buffer>;
}

export function assertReleaseState(
  api: any,
  expected: { repository?: string; version: string; sourceSha: string; releaseId: number },
  mode: "draft" | "published",
): Promise<any>;

export function createOrResolveDraft(
  api: any,
  input: { version: string; sourceSha: string; notes: string; resumeReleaseId?: string },
): Promise<any>;
export function downloadRelease(
  api: any,
  expected: { repository: string; version: string; sourceSha: string; releaseId: number },
  destination: string,
  mode: "draft" | "published",
): Promise<AssetRecord[]>;
export function validatePreflight(
  api: any,
  input: { version: string; sourceSha: string; resumeReleaseId?: string },
): Promise<any>;
