import assert from "node:assert/strict";
import { existsSync, mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import {
  HttpStatusError,
  ReleaseProtocolError,
  awaitExpectedAssets,
  awaitPublishedReleaseState,
  awaitVisibleRelease,
  buildReleaseIntent,
  buildStableUpdaterManifest,
  canonicalAssetUrl,
  coreAssetNames,
  downloadWithRetry,
  expectedAssetNames,
  sha256Bytes,
  uploadOrReconcile,
  validateReleaseMetadata,
  validatePublishedReleaseState,
  validateReleaseVerification,
  validateUpdaterManifest,
  verifyAttestationWithRetry,
} from "../scripts/release-state.mjs";
import { GitHubApi, assertReleaseState, createOrResolveDraft, downloadRelease, validatePreflight } from "../scripts/release-github.mjs";

function fakeClock() {
  let milliseconds = 0;
  return {
    clock: {
      now: () => milliseconds,
      sleep: async (delay: number) => { milliseconds += delay; },
    },
    elapsed: () => milliseconds,
  };
}

const sourceSha = "a".repeat(40);
const release = { id: 73, tag_name: "v0.2.1", target_commitish: sourceSha, draft: true, prerelease: false };
const expectedRelease = { releaseId: 73, version: "0.2.1", sourceSha };
const asset = {
  name: "codex-manager-v0.2.1-aarch64.app.tar.gz",
  size: 7,
  digest: `sha256:${"b".repeat(64)}`,
  stableUrl: "https://github.com/owner/repository/releases/download/v0.2.1/codex-manager-v0.2.1-aarch64.app.tar.gz",
};
const remoteAsset = { id: 99, name: asset.name, size: asset.size, digest: asset.digest, state: "uploaded" };

test("release visibility 可越过旧 20 秒窗口并在唯一 identity 后完成", async () => {
  const time = fakeClock();
  let calls = 0;
  const result = await awaitVisibleRelease(
    async () => (++calls <= 6 ? [] : [release]),
    expectedRelease,
    { clock: time.clock, deadlineMs: 300_000 },
  );
  assert.equal(result.id, 73);
  assert.ok(time.elapsed() > 20_000);
});

test("draft state 复验返回 exact Release 且完成 main/tag race 检查", async () => {
  const api = {
    getRelease: async () => release,
    getMainRef: async () => ({ object: { sha: sourceSha } }),
    getTagRef: async () => { throw new HttpStatusError(404); },
    listReleases: async () => [release],
  };
  const result = await assertReleaseState(api, expectedRelease, "draft");
  assert.equal(result.id, release.id);
});

test("asset partial 状态超过 20 秒后仍可收敛", async () => {
  const time = fakeClock();
  let calls = 0;
  const records = await awaitExpectedAssets(
    async () => (++calls <= 6 ? [] : [remoteAsset]),
    [asset],
    [asset.name],
    { clock: time.clock, deadlineMs: 300_000 },
  );
  assert.equal(records[0].id, 99);
  assert.ok(time.elapsed() > 20_000);
});

test("unknown asset 立即失败且不重试", async () => {
  const time = fakeClock();
  let calls = 0;
  await assert.rejects(
    awaitExpectedAssets(
      async () => {
        calls += 1;
        return [{ ...remoteAsset, name: "unknown.bin" }];
      },
      [asset],
      [asset.name],
      { clock: time.clock },
    ),
    (error: ReleaseProtocolError) => error.code === "unknown-asset",
  );
  assert.equal(calls, 1);
  assert.equal(time.elapsed(), 0);
});

test("attestation 仅对传播延迟重试", async () => {
  const time = fakeClock();
  let calls = 0;
  const result = await verifyAttestationWithRetry(
    async () => {
      calls += 1;
      if (calls < 3) throw new HttpStatusError(404, "attestation not visible");
      return { verified: true };
    },
    { clock: time.clock },
  );
  assert.deepEqual(result, { verified: true });
  assert.equal(calls, 3);
  await assert.rejects(
    verifyAttestationWithRetry(async () => { throw new HttpStatusError(403); }, { clock: time.clock }),
    (error: HttpStatusError) => error.status === 403,
  );
});

test("上传响应丢失后按 name/size/digest 调和且不重复上传", async () => {
  const time = fakeClock();
  let lists = 0;
  let uploads = 0;
  const result = await uploadOrReconcile({
    listAssets: async () => (++lists < 3 ? [] : [remoteAsset]),
    upload: async () => {
      uploads += 1;
      throw Object.assign(new Error("connection reset"), { code: "ECONNRESET" });
    },
    expected: asset,
    allowedNames: [asset.name],
    options: { clock: time.clock },
  });
  assert.equal(result.id, 99);
  assert.equal(uploads, 1);
});

test("同名资产 size 或 digest 不同立即失败", async () => {
  const time = fakeClock();
  await assert.rejects(
    uploadOrReconcile({
      listAssets: async () => [{ ...remoteAsset, size: 8 }],
      upload: async () => remoteAsset,
      expected: asset,
      allowedNames: [asset.name],
      options: { clock: time.clock },
    }),
    (error: ReleaseProtocolError) => error.code === "asset-size-mismatch",
  );
});

test("下载对 404/5xx/网络错误有界重试并原子落盘", async () => {
  const root = mkdtempSync(join(tmpdir(), "codex-manager-release-download-"));
  const bytes = Buffer.from("payload");
  const expected = { ...asset, size: bytes.length, digest: `sha256:${"239f59ed55e737c77147cf55ad0c1b030b6d7ee748a7426952f9b852d5a935e5"}` };
  const time = fakeClock();
  let calls = 0;
  try {
    const destination = join(root, asset.name);
    await downloadWithRetry({
      download: async () => {
        calls += 1;
        if (calls === 1) throw new HttpStatusError(404);
        if (calls === 2) throw new HttpStatusError(503);
        if (calls === 3) throw Object.assign(new Error("network reset"), { code: "ECONNRESET" });
        return bytes;
      },
      expected,
      destination,
      options: { clock: time.clock },
    });
    assert.deepEqual(readFileSync(destination), bytes);
    assert.equal(calls, 4);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("下载 digest 不符时不留下最终文件", async () => {
  const root = mkdtempSync(join(tmpdir(), "codex-manager-release-download-"));
  const destination = join(root, asset.name);
  try {
    await assert.rejects(
      downloadWithRetry({ download: async () => Buffer.from("wrong"), expected: asset, destination }),
      (error: ReleaseProtocolError) => error.code === "download-size-mismatch" || error.code === "download-digest-mismatch",
    );
    assert.equal(existsSync(destination), false);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("latest.json 只生成确定性 version-tag URL 并拒绝 untagged", () => {
  const manifest = buildStableUpdaterManifest({
    repository: "owner/repository",
    version: "0.2.1",
    assetName: asset.name,
    signature: "synthetic-updater-signature",
    publishedAt: "2026-08-29T12:00:00Z",
    notes: "synthetic notes",
  });
  assert.equal(manifest.platforms["darwin-aarch64"].url, canonicalAssetUrl("owner/repository", "0.2.1", asset.name));
  validateUpdaterManifest(manifest, {
    repository: "owner/repository",
    version: "0.2.1",
    assetName: asset.name,
    signature: "synthetic-updater-signature",
  });
  manifest.platforms["darwin-aarch64"].url = "https://github.com/owner/repository/releases/download/untagged-deadbeef/payload";
  assert.throws(
    () => validateUpdaterManifest(manifest, {
      repository: "owner/repository",
      version: "0.2.1",
      assetName: asset.name,
      signature: "synthetic-updater-signature",
    }),
    (error: ReleaseProtocolError) => error.code === "invalid-updater-manifest",
  );
});

test("post-publish 必须同时锁定 release/latest/tag SHA", () => {
  const published = { ...release, draft: false, published_at: "2026-08-29T12:00:00Z" };
  assert.equal(
    validatePublishedReleaseState({ release: published, latestRelease: published, tagSha: sourceSha, expected: expectedRelease }),
    true,
  );
  assert.throws(
    () => validatePublishedReleaseState({ release: published, latestRelease: { id: 74 }, tagSha: sourceSha, expected: expectedRelease }),
    (error: ReleaseProtocolError) => error.code === "latest-release-mismatch",
  );
});

test("post-publish 可等待 Release/latest 状态转换超过旧 20 秒窗口", async () => {
  const time = fakeClock();
  let calls = 0;
  const published = { ...release, draft: false, published_at: "2026-08-29T12:00:00Z" };
  const result = await awaitPublishedReleaseState(
    async () => {
      calls += 1;
      return calls <= 6
        ? { release: { ...release, draft: true }, latestRelease: { id: 12 }, tagSha: sourceSha }
        : { release: published, latestRelease: published, tagSha: sourceSha };
    },
    expectedRelease,
    { clock: time.clock, deadlineMs: 300_000 },
  );
  assert.equal(result.id, release.id);
  assert.ok(time.elapsed() > 20_000);
});

test("draft 创建响应丢失后只调和同 tag、同 source 的唯一 Release", async () => {
  let listCalls = 0;
  let createCalls = 0;
  const api = {
    repository: "owner/repository",
    getMainRef: async () => ({ object: { sha: sourceSha } }),
    getTagRef: async () => { throw new HttpStatusError(404); },
    listReleases: async () => (++listCalls === 1 ? [] : [release]),
    createRelease: async () => {
      createCalls += 1;
      throw Object.assign(new Error("connection reset after POST"), { code: "ECONNRESET" });
    },
  };
  const resolved = await createOrResolveDraft(api as never, {
    version: "0.2.1",
    sourceSha,
    notes: "synthetic notes",
  });
  assert.equal(resolved.id, release.id);
  assert.equal(createCalls, 1);
});

test("preflight 对 main、tag 和 existing Release identity fail closed", async () => {
  const cleanApi = {
    repository: "owner/repository",
    getMainRef: async () => ({ object: { sha: sourceSha } }),
    getTagRef: async () => { throw new HttpStatusError(404); },
    listReleases: async () => [],
  };
  assert.equal((await validatePreflight(cleanApi as never, { version: "0.2.1", sourceSha })).releaseAbsent, true);
  await assert.rejects(
    validatePreflight({ ...cleanApi, getMainRef: async () => ({ object: { sha: "b".repeat(40) } }) } as never, { version: "0.2.1", sourceSha }),
    (error: ReleaseProtocolError) => error.code === "main-source-mismatch",
  );
  await assert.rejects(
    validatePreflight({ ...cleanApi, listReleases: async () => [release] } as never, { version: "0.2.1", sourceSha }),
    (error: ReleaseProtocolError) => error.code === "release-conflict",
  );
});

test("published 回下载只使用稳定 tag URL，并额外验证 latest alias bytes", async () => {
  const root = mkdtempSync(join(tmpdir(), "codex-manager-published-download-"));
  const repository = "owner/repository";
  const version = "0.2.1";
  const names = expectedAssetNames(version);
  const bytesByName = new Map(names.map((name) => [name, Buffer.from(`synthetic:${name}`)]));
  const remoteAssets = names.map((name, index) => {
    const bytes = bytesByName.get(name)!;
    return { id: index + 1, name, size: bytes.length, state: "uploaded", digest: `sha256:${sha256Bytes(bytes)}` };
  });
  const published = { ...release, draft: false, published_at: "2026-08-29T12:00:00Z" };
  const urls: string[] = [];
  const api = {
    repository,
    getRelease: async () => published,
    getLatestRelease: async () => published,
    getTagRef: async () => ({ object: { type: "commit", sha: sourceSha } }),
    listAssets: async () => remoteAssets,
    downloadPublic: async (url: string) => {
      urls.push(url);
      if (url.endsWith("/releases/latest/download/latest.json")) return bytesByName.get("latest.json")!;
      const name = decodeURIComponent(url.slice(url.lastIndexOf("/") + 1));
      return bytesByName.get(name)!;
    },
  };
  try {
    const records = await downloadRelease(api as never, { repository, version, sourceSha, releaseId: release.id }, root, "published");
    assert.equal(records.length, names.length);
    assert.equal(urls.filter((url) => url.includes("/releases/download/v0.2.1/")).length, names.length);
    assert.equal(urls.some((url) => url.includes("/untagged-")), false);
    assert.equal(urls.at(-1), `https://github.com/${repository}/releases/latest/download/latest.json`);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("published public downloader 即使环境有 token 也不发送 Authorization", async () => {
  const originalFetch = globalThis.fetch;
  let observedHeaders: Headers | undefined;
  globalThis.fetch = (async (_input: string | URL | Request, init?: RequestInit) => {
    observedHeaders = new Headers(init?.headers);
    return new Response(Buffer.from("public bytes"), { status: 200 });
  }) as typeof fetch;
  try {
    const api = new GitHubApi("owner/repository", "synthetic-token-never-sent");
    assert.deepEqual(await api.downloadPublic("https://github.com/owner/repository/releases/download/v0.2.1/asset"), Buffer.from("public bytes"));
    assert.equal(observedHeaders?.has("Authorization"), false);
    assert.equal(observedHeaders?.get("X-GitHub-Api-Version"), "2022-11-28");
  } finally {
    globalThis.fetch = originalFetch;
  }
});

test("release intent 与 provenance v2 严格绑定 toolchain、签名证据和资产记录", () => {
  const repository = "owner/repository";
  const version = "0.2.1";
  const buildInput = {
    schemaVersion: 2,
    version,
    source: { repository, sha: sourceSha, ref: "refs/heads/main", workflowRef: `${repository}/.github/workflows/release.yml@refs/heads/main`, runId: "123", runAttempt: "1" },
    target: "aarch64-apple-darwin",
    runner: { imageOS: "macos15", imageVersion: "20260820.1", macOSVersion: "15.6", xcodeVersion: "Xcode 16.4" },
    toolchain: { node: "v24.8.0", npm: "11.5.0", rustc: "rustc 1.98.0", cargo: "cargo 1.98.0", tauri: "tauri-cli 2.11.4", gh: "gh 2.80.0" },
  };
  const appCdHash = "c".repeat(40);
  const signingEvidence = {
    schemaVersion: 2,
    version,
    signedAt: "2026-08-29T12:00:00Z",
    source: { repository, sha: sourceSha, ref: "refs/heads/main", workflowRef: `${repository}/.github/workflows/release.yml@refs/heads/main`, runId: "123", runAttempt: "1" },
    target: { triple: "aarch64-apple-darwin", architecture: "arm64", bundleIdentifier: "cc.codex.manager", bundleVersion: version, bundleBuild: version },
    signingRunner: { imageOS: "macos15", imageVersion: "20260820.1", macOSVersion: "15.6", xcodeVersion: "Xcode 16.4", nodeVersion: "v24.8.0", ghVersion: "gh 2.80.0", notarytoolVersion: "notarytool 1.0" },
    apple: {
      teamId: "ABCDE12345",
      authoritySha256: "d".repeat(64),
      leafCertificateSha256: "e".repeat(64),
      app: { cdHash: appCdHash, notarization: { id: "11111111-1111-1111-1111-111111111111", status: "Accepted", message: "Accepted" }, hardenedRuntime: true, secureTimestamp: true, forbiddenEntitlementsAbsent: true, stapled: true, gatekeeperSource: "Notarized Developer ID" },
      dmg: { cdHash: "f".repeat(40), embeddedAppCdHash: appCdHash, notarization: { id: "22222222-2222-2222-2222-222222222222", status: "Accepted", message: "Accepted" }, secureTimestamp: true, stapled: true, gatekeeperSource: "Notarized Developer ID" },
    },
    updater: { publicKeySha256: "1".repeat(64), embeddedAppCdHash: appCdHash },
  };
  const allRecords = expectedAssetNames(version).map((name, index) => ({
    id: index + 1,
    name,
    size: index + 10,
    digest: `sha256:${String((index + 2) % 10).repeat(64)}`,
    stableUrl: canonicalAssetUrl(repository, version, name),
  }));
  const coreRecords = allRecords.filter((record) => coreAssetNames(version).includes(record.name));
  const intent = buildReleaseIntent({
    repository,
    version,
    sourceSha,
    signingRunId: "123",
    signingRunAttempt: "1",
    buildInputSha256: "2".repeat(64),
    signingEvidenceSha256: "3".repeat(64),
    coreAssets: coreRecords.map(({ id: _id, ...record }) => record),
  });
  assert.equal(intent.coreAssets.length, 6);
  const provenance = {
    schemaVersion: 2,
    version,
    source: signingEvidence.source,
    release: { id: 73, tag: "v0.2.1", draft: true, canonicalBaseUrl: `https://github.com/${repository}/releases/download/v0.2.1` },
    buildInput,
    signing: signingEvidence,
    coreAssets: coreRecords,
  };
  const signature = "synthetic-updater-signature";
  const latest = buildStableUpdaterManifest({ repository, version, assetName: asset.name, signature, publishedAt: "2026-08-29T12:00:00Z", notes: "notes" });
  assert.equal(validateReleaseMetadata({
    latest,
    provenance,
    signingEvidence,
    buildInput,
    assetRecords: allRecords,
    signature,
    expected: { repository, version, sourceSha, releaseId: 73 },
  }), true);
  const verification = {
    schemaVersion: 1,
    status: "draft-remotely-verified",
    verifiedAt: "2026-08-29T12:00:00Z",
    source: { repository, sha: sourceSha, ref: "refs/heads/main" },
    verificationRun: { workflowRef: `${repository}/.github/workflows/release.yml@refs/heads/main`, workflowSha: sourceSha, runId: "123", runAttempt: "1" },
    release: { id: 73, tag: "v0.2.1", mode: "draft" },
    assets: allRecords,
    provenanceSha256: allRecords.find((record) => record.name === "BUILD-PROVENANCE.json")!.digest.slice("sha256:".length),
    attestations: { files: expectedAssetNames(version).map((name) => ({ name: `${name}.attestation.json`, sha256: "b".repeat(64) })) },
    checks: { exactAssetSetAndDigests: true, updaterSignatureAndStableUrl: true, appAndDmgTrustChain: true, sbomAndLicenses: true, anonymousStableDownloads: false, latestAlias: false },
  };
  assert.equal(validateReleaseVerification(verification, { repository, version, sourceSha, releaseId: 73, mode: "draft" }), verification);
  const contradictoryVerification = structuredClone(verification);
  contradictoryVerification.provenanceSha256 = "a".repeat(64);
  assert.throws(
    () => validateReleaseVerification(contradictoryVerification, { repository, version, sourceSha, releaseId: 73, mode: "draft" }),
    /provenanceSha256 differs/,
  );
  const invalid = structuredClone(provenance);
  invalid.buildInput.toolchain.node = "";
  assert.throws(
    () => validateReleaseMetadata({ latest, provenance: invalid, signingEvidence, buildInput: invalid.buildInput, assetRecords: allRecords, signature, expected: { repository, version, sourceSha, releaseId: 73 } }),
    (error: ReleaseProtocolError) => error.code === "invalid-schema",
  );
});
