export interface Clock {
  now(): number;
  sleep(milliseconds: number): Promise<void>;
}

export interface RetryOptions {
  clock?: Clock;
  deadlineMs?: number;
  deadlineAt?: number;
  initialDelayMs?: number;
  maxDelayMs?: number;
  label?: string;
}

export interface AssetRecord {
  id?: number;
  name: string;
  size: number;
  digest: string;
  stableUrl: string;
}

export interface RemoteAsset {
  id: number;
  name: string;
  size: number;
  digest: string;
  state: string;
}

export class ReleaseProtocolError extends Error {
  code: string;
  status?: number;
  transient: boolean;
}

export class HttpStatusError extends ReleaseProtocolError {
  constructor(status: number, message?: string);
}

export function awaitExpectedAssets(
  listAssets: () => Promise<RemoteAsset[]>,
  expectedRecords: AssetRecord[],
  allowedNames: string[],
  options?: RetryOptions,
): Promise<AssetRecord[]>;
export function awaitPublishedReleaseState(
  readState: () => Promise<any>,
  expected: Record<string, unknown>,
  options?: RetryOptions,
): Promise<any>;
export function awaitVisibleRelease(
  listReleases: () => Promise<unknown[]>,
  expected: Record<string, unknown>,
  options?: RetryOptions,
): Promise<any>;
export function buildReleaseIntent(input: Record<string, unknown>): any;
export function buildStableUpdaterManifest(input: Record<string, unknown>): any;
export function canonicalAssetUrl(repository: string, version: string, filename: string): string;
export function compareAssetRecordNames(left: Pick<AssetRecord, "name">, right: Pick<AssetRecord, "name">): number;
export function coreAssetNames(version: string): string[];
export function downloadWithRetry(input: {
  download: () => Promise<Buffer>;
  expected: AssetRecord;
  destination: string;
  options?: RetryOptions;
}): Promise<string>;
export function expectedAssetNames(version: string): string[];
export function sha256Bytes(bytes: Buffer | string): string;
export function uploadOrReconcile(input: {
  listAssets: () => Promise<RemoteAsset[]>;
  upload: () => Promise<RemoteAsset>;
  expected: AssetRecord;
  allowedNames: string[];
  options?: RetryOptions;
}): Promise<AssetRecord>;
export function validatePublishedReleaseState(input: Record<string, unknown>): true;
export function validateReleaseMetadata(input: Record<string, unknown>): true;
export function validateReleaseVerification(input: any, expected: Record<string, unknown>): any;
export function validateUpdaterManifest(manifest: any, expected: Record<string, unknown>): any;
export function verifyAttestationWithRetry<T>(verify: () => Promise<T>, options?: RetryOptions): Promise<T>;
