#!/usr/bin/env node

import { execFile } from "node:child_process";
import { promisify } from "node:util";
import { readdir, readFile, rename, rm, writeFile, mkdir } from "node:fs/promises";
import { basename, dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import {
  API_DIGEST,
  DEFAULT_DEADLINE_MS,
  HttpStatusError,
  ReleaseProtocolError,
  awaitExpectedAssets,
  awaitPublishedReleaseState,
  awaitVisibleRelease,
  buildStableUpdaterManifest,
  canonicalAssetUrl,
  canonicalTag,
  downloadWithRetry,
  expectedAssetNames,
  fileRecord,
  isTransientError,
  retryUntil,
  sha256Bytes,
  uploadOrReconcile,
  validateReleaseIdentity,
  verifyAttestationWithRetry,
} from "./release-state.mjs";

const execFileAsync = promisify(execFile);
const API_VERSION = "2022-11-28";
const REQUEST_TIMEOUT_MS = 60_000;

function requiredEnvironment(name) {
  const value = process.env[name];
  if (!value) throw new ReleaseProtocolError("missing-environment", `required environment variable is unavailable: ${name}`);
  return value;
}

function parseArgs(argv) {
  const command = argv[0];
  const values = new Map();
  for (let index = 1; index < argv.length; index += 1) {
    const key = argv[index];
    if (!key.startsWith("--") || index + 1 >= argv.length) {
      throw new ReleaseProtocolError("invalid-command", `invalid command argument: ${key ?? "<missing>"}`);
    }
    values.set(key.slice(2), argv[index + 1]);
    index += 1;
  }
  return {
    command,
    get(name, { required = true } = {}) {
      const value = values.get(name);
      if (required && !value) throw new ReleaseProtocolError("invalid-command", `missing --${name}`);
      return value;
    },
  };
}

async function atomicJson(path, value) {
  await mkdir(dirname(resolve(path)), { recursive: true });
  const partial = `${path}.partial-${process.pid}`;
  await rm(partial, { force: true });
  try {
    await writeFile(partial, `${JSON.stringify(value, null, 2)}\n`, { flag: "wx", mode: 0o600 });
    await rename(partial, path);
  } catch (error) {
    await rm(partial, { force: true });
    throw error;
  }
}

class GitHubApi {
  constructor(repository, token = process.env.GH_TOKEN) {
    this.repository = repository;
    this.token = token;
  }

  async request(urlOrPath, { method = "GET", body, headers = {}, authenticated = true, response = "json" } = {}) {
    if (authenticated && !this.token) throw new ReleaseProtocolError("missing-token", "GH_TOKEN is required for the authenticated release operation");
    const url = urlOrPath.startsWith("https://") ? urlOrPath : `https://api.github.com/${urlOrPath.replace(/^\//, "")}`;
    const requestHeaders = {
      Accept: "application/vnd.github+json",
      "User-Agent": "codex-manager-release-control",
      "X-GitHub-Api-Version": API_VERSION,
      ...headers,
    };
    if (authenticated) requestHeaders.Authorization = `Bearer ${this.token}`;
    const result = await fetch(url, {
      method,
      headers: requestHeaders,
      body,
      redirect: "follow",
      signal: AbortSignal.timeout(REQUEST_TIMEOUT_MS),
    });
    if (!result.ok) throw new HttpStatusError(result.status);
    if (response === "bytes") return Buffer.from(await result.arrayBuffer());
    if (response === "text") return result.text();
    return result.json();
  }

  async paginated(path) {
    const values = [];
    for (let page = 1; page <= 100; page += 1) {
      const separator = path.includes("?") ? "&" : "?";
      const batch = await this.request(`${path}${separator}per_page=100&page=${page}`);
      if (!Array.isArray(batch)) throw new ReleaseProtocolError("invalid-github-response", "paginated GitHub response is not an array");
      values.push(...batch);
      if (batch.length < 100) return values;
    }
    throw new ReleaseProtocolError("pagination-limit", "GitHub pagination exceeded 100 pages");
  }

  listReleases() {
    return this.paginated(`repos/${this.repository}/releases`);
  }

  listAssets(releaseId) {
    return this.paginated(`repos/${this.repository}/releases/${releaseId}/assets`);
  }

  getRelease(releaseId) {
    return this.request(`repos/${this.repository}/releases/${releaseId}`);
  }

  getLatestRelease() {
    return this.request(`repos/${this.repository}/releases/latest`);
  }

  getTagRef(tag) {
    return this.request(`repos/${this.repository}/git/ref/tags/${encodeURIComponent(tag)}`);
  }

  getAnnotatedTag(sha) {
    return this.request(`repos/${this.repository}/git/tags/${sha}`);
  }

  getMainRef() {
    return this.request(`repos/${this.repository}/git/ref/heads/main`);
  }

  createRelease(payload) {
    return this.request(`repos/${this.repository}/releases`, {
      method: "POST",
      body: JSON.stringify(payload),
      headers: { "Content-Type": "application/json" },
    });
  }

  uploadAsset(releaseId, record, bytes) {
    const url = `https://uploads.github.com/repos/${this.repository}/releases/${releaseId}/assets?name=${encodeURIComponent(record.name)}`;
    return this.request(url, {
      method: "POST",
      body: bytes,
      headers: { "Content-Type": "application/octet-stream" },
    });
  }

  downloadAssetById(assetId) {
    return this.request(`repos/${this.repository}/releases/assets/${assetId}`, {
      headers: { Accept: "application/octet-stream" },
      response: "bytes",
    });
  }

  downloadPublic(url) {
    return this.request(url, { authenticated: false, response: "bytes", headers: { Accept: "application/octet-stream" } });
  }
}

function releaseExpectation(repository, version, sourceSha, releaseId) {
  if (!/^[a-f0-9]{40}$/.test(sourceSha)) throw new ReleaseProtocolError("invalid-source-sha", "SOURCE_SHA must be a full lowercase Git SHA");
  const numericReleaseId = Number(releaseId);
  if (!Number.isSafeInteger(numericReleaseId) || numericReleaseId <= 0) throw new ReleaseProtocolError("invalid-release-id", "release ID must be a positive safe integer");
  return { repository, version, sourceSha, releaseId: numericReleaseId };
}

async function retryRequest(operation, label) {
  return retryUntil(operation, (value) => ({ kind: "ready", value }), { deadlineMs: DEFAULT_DEADLINE_MS, label });
}

async function assertMain(api, sourceSha) {
  const ref = await retryRequest(() => api.getMainRef(), "remote main lookup");
  if (ref?.object?.sha !== sourceSha) throw new ReleaseProtocolError("main-source-mismatch", "remote main no longer matches SOURCE_SHA");
}

async function tagExists(api, tag) {
  return retryUntil(
    async () => {
      try {
        await api.getTagRef(tag);
        return true;
      } catch (error) {
        if (error?.status === 404) return false;
        throw error;
      }
    },
    (value) => ({ kind: "ready", value }),
    { deadlineMs: DEFAULT_DEADLINE_MS, label: "tag existence lookup" },
  );
}

async function resolveTagSha(api, tag) {
  let object = (await retryRequest(() => api.getTagRef(tag), "tag lookup")).object;
  for (let depth = 0; depth < 4; depth += 1) {
    if (object?.type === "commit" && /^[a-f0-9]{40}$/.test(object.sha)) return object.sha;
    if (object?.type !== "tag" || !/^[a-f0-9]{40}$/.test(object.sha)) break;
    object = (await retryRequest(() => api.getAnnotatedTag(object.sha), "annotated tag lookup")).object;
  }
  throw new ReleaseProtocolError("invalid-tag-target", "release tag does not resolve to a commit");
}

async function createOrResolveDraft(api, { version, sourceSha, notes, resumeReleaseId }) {
  await assertMain(api, sourceSha);
  const tag = canonicalTag(version);
  if (await tagExists(api, tag)) throw new ReleaseProtocolError("tag-conflict", `release tag already exists: ${tag}`);

  if (resumeReleaseId) {
    const expected = releaseExpectation(api.repository, version, sourceSha, resumeReleaseId);
    validateReleaseIdentity(await retryRequest(() => api.getRelease(resumeReleaseId), "resume release lookup"), expected);
    return awaitVisibleRelease(() => api.listReleases(), expected, { deadlineMs: DEFAULT_DEADLINE_MS });
  }

  const existing = (await retryRequest(() => api.listReleases(), "release uniqueness lookup")).filter((item) => item.tag_name === tag);
  if (existing.length !== 0) throw new ReleaseProtocolError("release-conflict", `release already exists: ${tag}`);

  let created;
  try {
    created = await api.createRelease({
      tag_name: tag,
      target_commitish: sourceSha,
      name: `Codex Manager ${tag}`,
      body: notes,
      draft: true,
      prerelease: false,
      generate_release_notes: false,
    });
  } catch (error) {
    if (!isTransientError(error)) throw error;
    const recovered = await retryUntil(
      () => api.listReleases(),
      (releases) => {
        const matches = releases.filter((item) => item.tag_name === tag);
        if (matches.length === 0) return { kind: "retry", reason: "ambiguous create response has not reconciled" };
        if (matches.length !== 1) throw new ReleaseProtocolError("release-conflict", "ambiguous create produced multiple matching releases");
        const candidate = matches[0];
        const expected = releaseExpectation(api.repository, version, sourceSha, candidate.id);
        validateReleaseIdentity(candidate, expected);
        return { kind: "ready", value: candidate };
      },
      { deadlineMs: DEFAULT_DEADLINE_MS, label: "ambiguous release creation reconciliation" },
    );
    return recovered;
  }
  const expected = releaseExpectation(api.repository, version, sourceSha, created.id);
  validateReleaseIdentity(created, expected);
  return awaitVisibleRelease(() => api.listReleases(), expected, { deadlineMs: DEFAULT_DEADLINE_MS });
}

async function validatePreflight(api, { version, sourceSha, resumeReleaseId }) {
  await assertMain(api, sourceSha);
  const tag = canonicalTag(version);
  if (await tagExists(api, tag)) throw new ReleaseProtocolError("tag-conflict", `release tag already exists: ${tag}`);
  if (resumeReleaseId) {
    const expected = releaseExpectation(api.repository, version, sourceSha, resumeReleaseId);
    validateReleaseIdentity(await retryRequest(() => api.getRelease(resumeReleaseId), "resume release lookup"), expected);
    return awaitVisibleRelease(() => api.listReleases(), expected, { deadlineMs: DEFAULT_DEADLINE_MS });
  }
  const matches = (await retryRequest(() => api.listReleases(), "release uniqueness lookup"))
    .filter((release) => release.tag_name === tag);
  if (matches.length !== 0) throw new ReleaseProtocolError("release-conflict", `release already exists: ${tag}`);
  return { tag, sourceSha, releaseAbsent: true };
}

async function localRecords(assetDirectory, repository, version) {
  const entries = await readdir(assetDirectory, { withFileTypes: true });
  if (entries.some((entry) => !entry.isFile())) throw new ReleaseProtocolError("unknown-local-asset", "release asset directory contains a non-file entry");
  const files = entries.map((entry) => entry.name).sort();
  const allowed = new Set(expectedAssetNames(version));
  for (const name of files) {
    if (!allowed.has(name)) throw new ReleaseProtocolError("unknown-local-asset", `unexpected local release asset: ${name}`);
  }
  return Promise.all(files.map((name) => fileRecord(join(assetDirectory, name), repository, version)));
}

async function ensureDraftIdentity(api, expected) {
  validateReleaseIdentity(await retryRequest(() => api.getRelease(expected.releaseId), "draft identity lookup"), expected);
  return awaitVisibleRelease(() => api.listReleases(), expected, { deadlineMs: DEFAULT_DEADLINE_MS });
}

async function syncAssets(api, expected, assetDirectory) {
  await ensureDraftIdentity(api, expected);
  const deadlineAt = Date.now() + DEFAULT_DEADLINE_MS;
  const records = await localRecords(assetDirectory, api.repository, expected.version);
  if (records.length === 0) throw new ReleaseProtocolError("missing-local-assets", "release asset directory is empty");
  const allowed = expectedAssetNames(expected.version);
  for (const record of records) {
    await uploadOrReconcile({
      listAssets: () => api.listAssets(expected.releaseId),
      upload: async () => api.uploadAsset(expected.releaseId, record, await readFile(join(assetDirectory, record.name))),
      expected: record,
      allowedNames: allowed,
      options: { deadlineAt },
    });
  }
  return awaitExpectedAssets(() => api.listAssets(expected.releaseId), records, allowed, { deadlineAt });
}

function completeRemoteRecords(remoteAssets, repository, version) {
  const expectedNames = expectedAssetNames(version);
  const actualNames = remoteAssets.map((asset) => asset.name).sort();
  const expectedSet = new Set(expectedNames);
  const unknown = actualNames.filter((name) => !expectedSet.has(name));
  if (unknown.length > 0) throw new ReleaseProtocolError("unknown-asset", `unexpected release asset: ${unknown[0]}`);
  if (actualNames.length !== expectedNames.length) throw new ReleaseProtocolError("asset-set-pending", "remote release asset set is incomplete", { transient: true });
  const seen = new Set();
  return remoteAssets.map((asset) => {
    if (seen.has(asset.name)) throw new ReleaseProtocolError("duplicate-asset", `duplicate release asset: ${asset.name}`);
    seen.add(asset.name);
    if (asset.state !== "uploaded" || !Number.isSafeInteger(Number(asset.id)) || Number(asset.id) <= 0 || Number(asset.size) < 0) {
      throw new ReleaseProtocolError("asset-not-ready", `remote release asset is not uploaded: ${asset.name}`);
    }
    if (typeof asset.digest !== "string" || !API_DIGEST.test(asset.digest)) {
      throw new ReleaseProtocolError("asset-digest-pending", `remote release asset digest is unavailable: ${asset.name}`, { transient: true });
    }
    return {
      id: Number(asset.id),
      name: asset.name,
      size: Number(asset.size),
      digest: asset.digest,
      stableUrl: canonicalAssetUrl(repository, version, asset.name),
    };
  }).sort((left, right) => left.name.localeCompare(right.name, "en"));
}

async function waitCompleteRemoteRecords(api, version, releaseId) {
  return retryUntil(
    () => api.listAssets(releaseId),
    (assets) => {
      try {
        return { kind: "ready", value: completeRemoteRecords(assets, api.repository, version) };
      } catch (error) {
        if (error.code === "asset-set-pending" || error.code === "asset-digest-pending" || error.code === "asset-not-ready") {
          return { kind: "retry", reason: error.message };
        }
        throw error;
      }
    },
    { deadlineMs: DEFAULT_DEADLINE_MS, label: "complete release asset visibility" },
  );
}

async function downloadRecords(api, records, destination, mode) {
  await mkdir(destination, { recursive: true });
  const deadlineAt = Date.now() + DEFAULT_DEADLINE_MS;
  for (const record of records) {
    await downloadWithRetry({
      download: () => mode === "published" ? api.downloadPublic(record.stableUrl) : api.downloadAssetById(record.id),
      expected: record,
      destination: join(destination, record.name),
      options: { deadlineAt },
    });
  }
}

async function assertReleaseState(api, expected, mode) {
  if (mode === "published") {
    return awaitPublishedReleaseState(
      async () => ({
        release: await api.getRelease(expected.releaseId),
        latestRelease: await api.getLatestRelease(),
        tagSha: await resolveTagSha(api, canonicalTag(expected.version)),
      }),
      expected,
      { deadlineMs: DEFAULT_DEADLINE_MS },
    );
  }

  const release = await retryRequest(() => api.getRelease(expected.releaseId), "release identity lookup");
  await assertMain(api, expected.sourceSha);
  validateReleaseIdentity(release, expected);
  if (await tagExists(api, canonicalTag(expected.version))) {
    throw new ReleaseProtocolError("tag-race", "release tag appeared during draft verification");
  }
  await awaitVisibleRelease(() => api.listReleases(), expected, { deadlineMs: DEFAULT_DEADLINE_MS });
  return release;
}

async function downloadRelease(api, expected, destination, mode) {
  await assertReleaseState(api, expected, mode);
  const records = await waitCompleteRemoteRecords(api, expected.version, expected.releaseId);
  await downloadRecords(api, records, destination, mode);
  if (mode === "published") {
    const latestUrl = `https://github.com/${api.repository}/releases/latest/download/latest.json`;
    const aliasBytes = await retryRequest(() => api.downloadPublic(latestUrl), "anonymous latest.json download");
    const taggedBytes = await readFile(join(destination, "latest.json"));
    if (!aliasBytes.equals(taggedBytes)) throw new ReleaseProtocolError("latest-alias-mismatch", "latest.json alias bytes differ from the version-tag asset");
  }
  return records;
}

function transientAttestationFailure(stderr) {
  return /(no attestations|not found|\b404\b|\b429\b|\b5\d\d\b|timeout|timed out|temporar|connection|network|EOF)/i.test(stderr ?? "");
}

async function verifyAttestation(assetPath, repository, sourceSha) {
  try {
    const { stdout } = await execFileAsync(
      "gh",
      [
        "attestation", "verify", assetPath,
        "--repo", repository,
        "--source-digest", sourceSha,
        "--source-ref", "refs/heads/main",
        "--signer-workflow", `${repository}/.github/workflows/release.yml`,
        "--signer-digest", sourceSha,
        "--deny-self-hosted-runners",
        "--predicate-type", "https://slsa.dev/provenance/v1",
        "--format", "json",
      ],
      { env: process.env, maxBuffer: 32 * 1024 * 1024 },
    );
    const parsed = JSON.parse(stdout);
    if (!Array.isArray(parsed) || parsed.length === 0) throw new ReleaseProtocolError("invalid-attestation", "attestation verification returned no results");
    return parsed;
  } catch (error) {
    if (error instanceof ReleaseProtocolError) throw error;
    if (transientAttestationFailure(error.stderr)) {
      throw new ReleaseProtocolError("attestation-pending", "attestation is not yet verifiable", { transient: true });
    }
    throw new ReleaseProtocolError("attestation-verification-failed", `attestation identity verification failed for ${basename(assetPath)}`);
  }
}

async function verifyAttestations(assetDirectory, evidenceDirectory, repository, sourceSha) {
  await mkdir(evidenceDirectory, { recursive: true });
  const deadlineAt = Date.now() + DEFAULT_DEADLINE_MS;
  const names = expectedAssetNames(requiredEnvironment("VERSION"));
  const files = [];
  for (const name of names) {
    const result = await verifyAttestationWithRetry(
      () => verifyAttestation(join(assetDirectory, name), repository, sourceSha),
      { deadlineAt, label: `attestation for ${name}` },
    );
    const evidenceName = `${name}.attestation.json`;
    const evidencePath = join(evidenceDirectory, evidenceName);
    await atomicJson(evidencePath, result);
    files.push({ name: evidenceName, sha256: sha256Bytes(await readFile(evidencePath)) });
  }
  return files;
}

async function main() {
  const args = parseArgs(process.argv.slice(2));
  const repository = requiredEnvironment("REPOSITORY");
  const version = requiredEnvironment("VERSION");
  const sourceSha = requiredEnvironment("SOURCE_SHA");
  const api = new GitHubApi(repository);

  if (args.command === "create-or-resolve-draft") {
    const notes = await readFile(args.get("notes"), "utf8");
    const resumeReleaseId = args.get("resume-release-id", { required: false });
    const release = await createOrResolveDraft(api, { version, sourceSha, notes, resumeReleaseId });
    await atomicJson(args.get("output"), release);
    return;
  }

  if (args.command === "make-latest") {
    const signature = (await readFile(args.get("signature-file"), "utf8")).replace(/[\r\n]/g, "");
    const manifest = buildStableUpdaterManifest({
      repository,
      version,
      assetName: `codex-manager-v${version}-aarch64.app.tar.gz`,
      signature,
      publishedAt: args.get("published-at"),
      notes: args.get("notes"),
    });
    await atomicJson(args.get("output"), manifest);
    return;
  }
  if (args.command === "verify-attestations") {
    const names = await verifyAttestations(args.get("asset-dir"), args.get("evidence-dir"), repository, sourceSha);
    await atomicJson(args.get("output"), { files: names });
    return;
  }
  if (args.command === "preflight") {
    const result = await validatePreflight(api, {
      version,
      sourceSha,
      resumeReleaseId: args.get("resume-release-id", { required: false }),
    });
    await atomicJson(args.get("output"), result);
    return;
  }

  const releaseId = args.get("release-id");
  const expected = releaseExpectation(repository, version, sourceSha, releaseId);
  if (args.command === "sync-assets") {
    const records = await syncAssets(api, expected, args.get("asset-dir"));
    await atomicJson(args.get("output"), records);
    return;
  }
  if (args.command === "download-assets") {
    await ensureDraftIdentity(api, expected);
    const local = await localRecords(args.get("asset-dir"), repository, version);
    if (JSON.stringify(local.map((item) => item.name).sort()) !== JSON.stringify(expectedAssetNames(version))) {
      throw new ReleaseProtocolError("local-asset-set-mismatch", "local final release asset set is incomplete");
    }
    const records = await awaitExpectedAssets(() => api.listAssets(releaseId), local, expectedAssetNames(version), { deadlineMs: DEFAULT_DEADLINE_MS });
    await downloadRecords(api, records, args.get("destination"), "draft");
    await atomicJson(args.get("output"), records);
    return;
  }
  if (args.command === "download-release") {
    const mode = args.get("mode");
    if (!new Set(["draft", "published"]).has(mode)) throw new ReleaseProtocolError("invalid-command", "--mode must be draft or published");
    const records = await downloadRelease(api, expected, args.get("destination"), mode);
    await atomicJson(args.get("output"), records);
    return;
  }
  if (args.command === "verify-state") {
    const mode = args.get("mode");
    if (!new Set(["draft", "published"]).has(mode)) throw new ReleaseProtocolError("invalid-command", "--mode must be draft or published");
    const release = await assertReleaseState(api, expected, mode);
    await atomicJson(args.get("output"), release);
    return;
  }
  throw new ReleaseProtocolError("invalid-command", `unsupported release command: ${args.command ?? "<missing>"}`);
}

export { GitHubApi, assertReleaseState, completeRemoteRecords, createOrResolveDraft, downloadRelease, syncAssets, transientAttestationFailure, validatePreflight };

if (resolve(process.argv[1] ?? "") === fileURLToPath(import.meta.url)) {
  main().catch((error) => {
    process.stderr.write(`release GitHub control failed: ${error.message}\n`);
    process.exitCode = 1;
  });
}
