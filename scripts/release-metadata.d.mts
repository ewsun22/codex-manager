import type { AssetRecord } from "./release-state.mjs";

export interface ReleaseMetadataArgs {
  get(name: string): string | undefined;
}

export interface ReleaseMetadataContext {
  repository: string;
  version: string;
  sourceSha: string;
}

export function coreRecords(assetDirectory: string, repository: string, version: string): Promise<AssetRecord[]>;
export function validateIntentFiles(args: ReleaseMetadataArgs, context: ReleaseMetadataContext): Promise<void>;
