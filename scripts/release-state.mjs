#!/usr/bin/env node

import { createHash } from "node:crypto";
import { mkdir, readFile, rename, rm, stat, writeFile } from "node:fs/promises";
import { basename, dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const DEFAULT_DEADLINE_MS = 5 * 60 * 1000;
const DEFAULT_INITIAL_DELAY_MS = 1_000;
const DEFAULT_MAX_DELAY_MS = 30_000;
const API_DIGEST = /^sha256:([a-f0-9]{64})$/;
const HEX_SHA256 = /^[a-f0-9]{64}$/;
const SOURCE_SHA = /^[a-f0-9]{40}$/;
const SEMVER = /^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(?:-[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?$/;
const RFC3339_UTC = /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z$/;

class ReleaseProtocolError extends Error {
  constructor(code, message, options = {}) {
    super(message, options);
    this.name = "ReleaseProtocolError";
    this.code = code;
    this.status = options.status;
    this.transient = options.transient === true;
  }
}

class HttpStatusError extends ReleaseProtocolError {
  constructor(status, message = `GitHub request failed with HTTP ${status}`) {
    super("github-http", message, { status, transient: isTransientStatus(status) });
  }
}

function assert(condition, code, message) {
  if (!condition) throw new ReleaseProtocolError(code, message);
}

function canonicalTag(version) {
  assert(typeof version === "string" && SEMVER.test(version), "invalid-version", "release version must be SemVer");
  return `v${version}`;
}

function canonicalAssetUrl(repository, version, filename) {
  assert(
    typeof repository === "string" && /^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/.test(repository),
    "invalid-repository",
    "repository must be owner/name",
  );
  assert(
    typeof filename === "string" && filename === basename(filename) && filename !== "." && filename !== "..",
    "invalid-asset-name",
    "asset name must be a basename",
  );
  return `https://github.com/${repository}/releases/download/${canonicalTag(version)}/${encodeURIComponent(filename)}`;
}

function expectedAssetNames(version) {
  canonicalTag(version);
  const stem = `codex-manager-v${version}`;
  return [
    `${stem}-aarch64.app.tar.gz`,
    `${stem}-aarch64.app.tar.gz.sig`,
    `${stem}-aarch64.dmg`,
    `${stem}-licenses.zip`,
    `${stem}-sbom.zip`,
    "BUILD-PROVENANCE.json",
    "SIGNING-EVIDENCE.json",
    `SHA256SUMS-v${version}-signed`,
    "latest.json",
  ].sort();
}

function coreAssetNames(version) {
  const metadata = new Set(["BUILD-PROVENANCE.json", `SHA256SUMS-v${version}-signed`, "latest.json"]);
  return expectedAssetNames(version).filter((name) => !metadata.has(name));
}

function sha256Bytes(bytes) {
  return createHash("sha256").update(bytes).digest("hex");
}

function compareAssetRecordNames(left, right) {
  return left.name < right.name ? -1 : left.name > right.name ? 1 : 0;
}

async function fileRecord(path, repository, version) {
  const metadata = await stat(path);
  assert(metadata.isFile(), "invalid-local-asset", `asset is not a regular file: ${basename(path)}`);
  const bytes = await readFile(path);
  const name = basename(path);
  return {
    name,
    size: bytes.length,
    digest: `sha256:${sha256Bytes(bytes)}`,
    stableUrl: canonicalAssetUrl(repository, version, name),
  };
}

function isTransientStatus(status) {
  return status === 404 || status === 408 || status === 425 || status === 429 || (status >= 500 && status <= 599);
}

function isTransientError(error) {
  if (error?.transient === true) return true;
  if (typeof error?.status === "number") return isTransientStatus(error.status);
  if (["AbortError", "TimeoutError", "TypeError"].includes(error?.name)) return true;
  return ["ECONNRESET", "ECONNREFUSED", "ETIMEDOUT", "EAI_AGAIN", "ENETUNREACH", "UND_ERR_CONNECT_TIMEOUT"].includes(
    error?.code,
  );
}

function realClock() {
  return {
    now: () => Date.now(),
    sleep: (milliseconds) => new Promise((resolveSleep) => setTimeout(resolveSleep, milliseconds)),
  };
}

async function retryUntil(operation, classify, options = {}) {
  const clock = options.clock ?? realClock();
  const deadlineMs = options.deadlineMs ?? DEFAULT_DEADLINE_MS;
  const startedAt = clock.now();
  const deadline = options.deadlineAt ?? startedAt + deadlineMs;
  let delay = options.initialDelayMs ?? DEFAULT_INITIAL_DELAY_MS;
  const maxDelay = options.maxDelayMs ?? DEFAULT_MAX_DELAY_MS;
  let attempt = 0;
  let lastReason = "remote state not ready";

  while (true) {
    attempt += 1;
    try {
      const value = await operation({ attempt, deadline });
      const decision = classify(value);
      if (decision.kind === "ready") return decision.value ?? value;
      if (decision.kind !== "retry") {
        throw new ReleaseProtocolError("invalid-retry-decision", "retry classifier returned an invalid decision");
      }
      lastReason = decision.reason ?? lastReason;
    } catch (error) {
      if (!isTransientError(error)) throw error;
      lastReason = error.message ?? "transient GitHub failure";
    }

    const now = clock.now();
    if (now >= deadline) {
      throw new ReleaseProtocolError("deadline-exceeded", `${options.label ?? "release operation"} exceeded deadline: ${lastReason}`);
    }
    const wait = Math.min(delay, maxDelay, deadline - now);
    await clock.sleep(wait);
    delay = Math.min(delay * 2, maxDelay);
  }
}

function validateReleaseIdentity(release, expected, { published = false } = {}) {
  assert(release && typeof release === "object", "invalid-release", "release response must be an object");
  assert(Number(release.id) === Number(expected.releaseId), "release-identity-mismatch", "release ID does not match");
  assert(release.tag_name === canonicalTag(expected.version), "release-identity-mismatch", "release tag does not match");
  assert(release.target_commitish === expected.sourceSha, "release-identity-mismatch", "release source SHA does not match");
  assert(release.prerelease === false, "release-identity-mismatch", "release must not be a prerelease");
  assert(release.draft === !published, "release-identity-mismatch", published ? "release is still a draft" : "release is not a draft");
  if (published) assert(typeof release.published_at === "string" && release.published_at.length > 0, "release-identity-mismatch", "published release timestamp is missing");
  return release;
}

function classifyReleaseVisibility(releases, expected) {
  assert(Array.isArray(releases), "invalid-release-list", "release list must be an array");
  const matches = releases.filter((release) => release.tag_name === canonicalTag(expected.version));
  if (matches.length === 0) return { kind: "retry", reason: "created release is not list-visible" };
  assert(matches.length === 1, "release-conflict", "multiple releases use the requested tag");
  validateReleaseIdentity(matches[0], expected);
  return { kind: "ready", value: matches[0] };
}

async function awaitVisibleRelease(listReleases, expected, options = {}) {
  return retryUntil(listReleases, (releases) => classifyReleaseVisibility(releases, expected), {
    ...options,
    label: options.label ?? "release visibility",
  });
}

function remoteDigest(asset) {
  assert(typeof asset.digest === "string" && API_DIGEST.test(asset.digest), "asset-digest-pending", `asset digest is unavailable: ${asset.name}`);
  return asset.digest;
}

function indexRemoteAssets(remoteAssets, allowedNames) {
  assert(Array.isArray(remoteAssets), "invalid-asset-list", "asset list must be an array");
  const allowed = new Set(allowedNames);
  const indexed = new Map();
  for (const asset of remoteAssets) {
    assert(asset && typeof asset.name === "string", "invalid-asset", "release asset is missing a name");
    assert(allowed.has(asset.name), "unknown-asset", `unexpected release asset: ${asset.name}`);
    assert(!indexed.has(asset.name), "duplicate-asset", `duplicate release asset: ${asset.name}`);
    indexed.set(asset.name, asset);
  }
  return indexed;
}

function validateRemoteAsset(asset, expected) {
  assert(Number.isSafeInteger(Number(asset.id)) && Number(asset.id) > 0, "invalid-asset", `asset ID is invalid: ${expected.name}`);
  if (asset.state !== "uploaded") return { kind: "retry", reason: `asset is not uploaded: ${expected.name}` };
  assert(Number(asset.size) === expected.size, "asset-size-mismatch", `asset size differs: ${expected.name}`);
  let digest;
  try {
    digest = remoteDigest(asset);
  } catch (error) {
    if (error.code === "asset-digest-pending") return { kind: "retry", reason: error.message };
    throw error;
  }
  assert(digest === expected.digest, "asset-digest-mismatch", `asset digest differs: ${expected.name}`);
  return {
    kind: "ready",
    value: {
      id: Number(asset.id),
      name: expected.name,
      size: expected.size,
      digest,
      stableUrl: expected.stableUrl,
    },
  };
}

function classifyAssetSet(remoteAssets, expectedRecords, allowedNames) {
  const indexed = indexRemoteAssets(remoteAssets, allowedNames);
  const records = [];
  const missing = [];
  const pending = [];
  for (const expected of expectedRecords) {
    const asset = indexed.get(expected.name);
    if (!asset) {
      missing.push(expected.name);
      continue;
    }
    const decision = validateRemoteAsset(asset, expected);
    if (decision.kind === "retry") pending.push(expected.name);
    else records.push(decision.value);
  }
  if (missing.length > 0 || pending.length > 0) {
    return { kind: "retry", reason: `assets pending; missing=${missing.join(",") || "none"}; pending=${pending.join(",") || "none"}` };
  }
  return { kind: "ready", value: records.sort(compareAssetRecordNames) };
}

async function awaitExpectedAssets(listAssets, expectedRecords, allowedNames, options = {}) {
  return retryUntil(listAssets, (assets) => classifyAssetSet(assets, expectedRecords, allowedNames), {
    ...options,
    label: options.label ?? "release asset visibility",
  });
}

async function uploadOrReconcile({ listAssets, upload, expected, allowedNames, options = {} }) {
  const classifyOne = (assets) => {
    const indexed = indexRemoteAssets(assets, allowedNames);
    const current = indexed.get(expected.name);
    if (!current) return { kind: "retry", reason: `asset is not list-visible: ${expected.name}` };
    return validateRemoteAsset(current, expected);
  };

  const initial = await retryUntil(listAssets, (value) => ({ kind: "ready", value }), {
    ...options,
    label: `initial asset lookup for ${expected.name}`,
  });
  const initialDecision = classifyOne(initial);
  if (initialDecision.kind === "ready") return initialDecision.value;

  try {
    const uploaded = await upload();
    const uploadDecision = validateRemoteAsset(uploaded, expected);
    if (uploadDecision.kind === "ready") {
      return retryUntil(listAssets, classifyOne, { ...options, label: `asset reconciliation for ${expected.name}` });
    }
  } catch (error) {
    const ambiguous = isTransientError(error) || [409, 422].includes(error?.status);
    if (!ambiguous) throw error;
  }

  return retryUntil(listAssets, classifyOne, { ...options, label: `ambiguous upload reconciliation for ${expected.name}` });
}

async function downloadWithRetry({ download, expected, destination, options = {} }) {
  const bytes = await retryUntil(
    download,
    (value) => {
      assert(Buffer.isBuffer(value), "invalid-download", `download did not return bytes: ${expected.name}`);
      assert(value.length === expected.size, "download-size-mismatch", `downloaded asset size differs: ${expected.name}`);
      assert(`sha256:${sha256Bytes(value)}` === expected.digest, "download-digest-mismatch", `downloaded asset digest differs: ${expected.name}`);
      return { kind: "ready", value };
    },
    { ...options, label: `asset download for ${expected.name}` },
  );
  await mkdir(dirname(destination), { recursive: true });
  const partial = `${destination}.partial-${process.pid}`;
  await rm(partial, { force: true });
  try {
    await writeFile(partial, bytes, { flag: "wx", mode: 0o600 });
    await rename(partial, destination);
  } catch (error) {
    await rm(partial, { force: true });
    throw error;
  }
  return destination;
}

async function verifyAttestationWithRetry(verify, options = {}) {
  return retryUntil(verify, (value) => ({ kind: "ready", value }), {
    ...options,
    label: options.label ?? "attestation propagation",
  });
}

function buildStableUpdaterManifest({ repository, version, assetName, signature, publishedAt, notes }) {
  assert(typeof signature === "string" && signature.length > 20 && !/[\r\n]/.test(signature), "invalid-updater-signature", "updater signature is invalid");
  assert(typeof publishedAt === "string" && RFC3339_UTC.test(publishedAt) && !Number.isNaN(Date.parse(publishedAt)), "invalid-pub-date", "updater pub_date must be RFC3339 UTC");
  const url = canonicalAssetUrl(repository, version, assetName);
  const platform = { signature, url };
  return {
    version,
    notes,
    pub_date: publishedAt,
    platforms: {
      "darwin-aarch64": { ...platform },
      "darwin-aarch64-app": { ...platform },
    },
  };
}

function exactKeys(value, expected, label) {
  assert(value && typeof value === "object" && !Array.isArray(value), "invalid-schema", `${label} must be an object`);
  const actual = Object.keys(value).sort();
  assert(JSON.stringify(actual) === JSON.stringify([...expected].sort()), "invalid-schema", `${label} fields do not match schema`);
}

function assertHexSha256(value, label) {
  assert(typeof value === "string" && HEX_SHA256.test(value), "invalid-schema", `${label} must be a SHA-256 digest`);
}

function assertNonEmptyString(value, label) {
  assert(typeof value === "string" && value.length > 0, "invalid-schema", `${label} must be a non-empty string`);
}

function validateBuildInput(buildInput, expected) {
  exactKeys(buildInput, ["schemaVersion", "version", "source", "target", "runner", "toolchain"], "BUILD-INPUT.json");
  assert(buildInput.schemaVersion === 2, "invalid-schema", "BUILD-INPUT.json schemaVersion must be 2");
  assert(buildInput.version === expected.version, "invalid-schema", "BUILD-INPUT.json version differs");
  exactKeys(buildInput.source, ["repository", "sha", "ref", "workflowRef", "runId", "runAttempt"], "BUILD-INPUT.json source");
  assert(buildInput.source.repository === expected.repository && buildInput.source.sha === expected.sourceSha, "invalid-schema", "BUILD-INPUT.json source identity differs");
  assert(buildInput.source.ref === "refs/heads/main", "invalid-schema", "BUILD-INPUT.json source ref differs");
  assert(buildInput.source.workflowRef === `${expected.repository}/.github/workflows/release.yml@refs/heads/main`, "invalid-schema", "BUILD-INPUT.json workflow ref differs");
  assert(/^[1-9][0-9]*$/.test(buildInput.source.runId) && /^[1-9][0-9]*$/.test(buildInput.source.runAttempt), "invalid-schema", "BUILD-INPUT.json run identity is invalid");
  assert(buildInput.target === "aarch64-apple-darwin", "invalid-schema", "BUILD-INPUT.json target differs");
  exactKeys(buildInput.runner, ["imageOS", "imageVersion", "macOSVersion", "xcodeVersion"], "BUILD-INPUT.json runner");
  exactKeys(buildInput.toolchain, ["node", "npm", "rustc", "cargo", "tauri", "gh"], "BUILD-INPUT.json toolchain");
  for (const [name, value] of Object.entries({ ...buildInput.runner, ...buildInput.toolchain })) assertNonEmptyString(value, `BUILD-INPUT.json ${name}`);
  return buildInput;
}

function validateNotarization(value, label) {
  exactKeys(value, ["id", "status", "message"], label);
  assert(/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i.test(value.id), "invalid-schema", `${label} id must be a UUID`);
  assert(value.status === "Accepted", "invalid-schema", `${label} status must be Accepted`);
  assert(typeof value.message === "string", "invalid-schema", `${label} message must be a string`);
}

function validateSigningEvidence(evidence, expected) {
  exactKeys(evidence, ["schemaVersion", "version", "signedAt", "source", "target", "signingRunner", "apple", "updater"], "SIGNING-EVIDENCE.json");
  assert(evidence.schemaVersion === 2, "invalid-schema", "SIGNING-EVIDENCE.json schemaVersion must be 2");
  assert(evidence.version === expected.version, "invalid-schema", "SIGNING-EVIDENCE.json version differs");
  assert(typeof evidence.signedAt === "string" && RFC3339_UTC.test(evidence.signedAt) && !Number.isNaN(Date.parse(evidence.signedAt)), "invalid-schema", "SIGNING-EVIDENCE.json signedAt is invalid");
  exactKeys(evidence.source, ["repository", "sha", "ref", "workflowRef", "runId", "runAttempt"], "SIGNING-EVIDENCE.json source");
  assert(evidence.source.repository === expected.repository && evidence.source.sha === expected.sourceSha, "invalid-schema", "SIGNING-EVIDENCE.json source identity differs");
  assert(evidence.source.ref === "refs/heads/main", "invalid-schema", "SIGNING-EVIDENCE.json source ref differs");
  assert(evidence.source.workflowRef === `${expected.repository}/.github/workflows/release.yml@refs/heads/main`, "invalid-schema", "SIGNING-EVIDENCE.json workflow ref differs");
  assert(/^[1-9][0-9]*$/.test(evidence.source.runId) && /^[1-9][0-9]*$/.test(evidence.source.runAttempt), "invalid-schema", "SIGNING-EVIDENCE.json run identity is invalid");
  exactKeys(evidence.target, ["triple", "architecture", "bundleIdentifier", "bundleVersion", "bundleBuild"], "SIGNING-EVIDENCE.json target");
  assert(evidence.target.triple === "aarch64-apple-darwin" && evidence.target.architecture === "arm64", "invalid-schema", "SIGNING-EVIDENCE.json architecture differs");
  assert(evidence.target.bundleIdentifier === "cc.codex.manager", "invalid-schema", "SIGNING-EVIDENCE.json bundle identifier differs");
  assert(evidence.target.bundleVersion === expected.version && evidence.target.bundleBuild === expected.version, "invalid-schema", "SIGNING-EVIDENCE.json bundle version differs");
  exactKeys(evidence.signingRunner, ["imageOS", "imageVersion", "macOSVersion", "xcodeVersion", "nodeVersion", "ghVersion", "notarytoolVersion"], "SIGNING-EVIDENCE.json signingRunner");
  for (const [name, value] of Object.entries(evidence.signingRunner)) assertNonEmptyString(value, `SIGNING-EVIDENCE.json signingRunner ${name}`);
  exactKeys(evidence.apple, ["teamId", "authoritySha256", "leafCertificateSha256", "app", "dmg"], "SIGNING-EVIDENCE.json apple");
  assert(/^[A-Z0-9]{10}$/.test(evidence.apple.teamId), "invalid-schema", "SIGNING-EVIDENCE.json Team ID is invalid");
  assertHexSha256(evidence.apple.authoritySha256, "SIGNING-EVIDENCE.json authoritySha256");
  assertHexSha256(evidence.apple.leafCertificateSha256, "SIGNING-EVIDENCE.json leafCertificateSha256");
  exactKeys(evidence.apple.app, ["cdHash", "notarization", "hardenedRuntime", "secureTimestamp", "forbiddenEntitlementsAbsent", "stapled", "gatekeeperSource"], "SIGNING-EVIDENCE.json app");
  assert(/^[a-f0-9]{40}$/.test(evidence.apple.app.cdHash), "invalid-schema", "SIGNING-EVIDENCE.json app CDHash is invalid");
  validateNotarization(evidence.apple.app.notarization, "SIGNING-EVIDENCE.json app notarization");
  for (const name of ["hardenedRuntime", "secureTimestamp", "forbiddenEntitlementsAbsent", "stapled"]) {
    assert(evidence.apple.app[name] === true, "invalid-schema", `SIGNING-EVIDENCE.json app ${name} must be true`);
  }
  assert(evidence.apple.app.gatekeeperSource === "Notarized Developer ID", "invalid-schema", "SIGNING-EVIDENCE.json app Gatekeeper source differs");
  exactKeys(evidence.apple.dmg, ["cdHash", "embeddedAppCdHash", "notarization", "secureTimestamp", "stapled", "gatekeeperSource"], "SIGNING-EVIDENCE.json dmg");
  assert(/^[a-f0-9]{40}$/.test(evidence.apple.dmg.cdHash), "invalid-schema", "SIGNING-EVIDENCE.json DMG CDHash is invalid");
  assert(evidence.apple.dmg.embeddedAppCdHash === evidence.apple.app.cdHash, "invalid-schema", "SIGNING-EVIDENCE.json embedded app CDHash differs");
  validateNotarization(evidence.apple.dmg.notarization, "SIGNING-EVIDENCE.json DMG notarization");
  assert(evidence.apple.dmg.notarization.id !== evidence.apple.app.notarization.id, "invalid-schema", "app and DMG notarization submissions must be distinct");
  assert(evidence.apple.dmg.secureTimestamp === true && evidence.apple.dmg.stapled === true, "invalid-schema", "SIGNING-EVIDENCE.json DMG signature evidence is incomplete");
  assert(evidence.apple.dmg.gatekeeperSource === "Notarized Developer ID", "invalid-schema", "SIGNING-EVIDENCE.json DMG Gatekeeper source differs");
  exactKeys(evidence.updater, ["publicKeySha256", "embeddedAppCdHash"], "SIGNING-EVIDENCE.json updater");
  assertHexSha256(evidence.updater.publicKeySha256, "SIGNING-EVIDENCE.json updater public key");
  assert(evidence.updater.embeddedAppCdHash === evidence.apple.app.cdHash, "invalid-schema", "SIGNING-EVIDENCE.json updater app CDHash differs");
  return evidence;
}

function validateAssetRecord(record, expected, { requireId = true } = {}) {
  exactKeys(record, requireId ? ["id", "name", "size", "digest", "stableUrl"] : ["name", "size", "digest", "stableUrl"], "asset record");
  if (requireId) assert(Number.isSafeInteger(Number(record.id)) && Number(record.id) > 0, "invalid-schema", `asset ID is invalid: ${record.name}`);
  assert(record.name === expected.name, "invalid-schema", `asset name differs: ${expected.name}`);
  assert(Number.isSafeInteger(Number(record.size)) && Number(record.size) >= 0, "invalid-schema", `asset size is invalid: ${record.name}`);
  assert(typeof record.digest === "string" && API_DIGEST.test(record.digest), "invalid-schema", `asset digest is invalid: ${record.name}`);
  assert(record.stableUrl === canonicalAssetUrl(expected.repository, expected.version, record.name), "invalid-schema", `asset stable URL differs: ${record.name}`);
  return record;
}

function buildReleaseIntent({ repository, version, sourceSha, signingRunId, signingRunAttempt, buildInputSha256, signingEvidenceSha256, coreAssets }) {
  const intent = {
    schemaVersion: 1,
    repository,
    version,
    tag: canonicalTag(version),
    sourceSha,
    signingRun: { id: String(signingRunId), attempt: String(signingRunAttempt) },
    buildInputSha256,
    signingEvidenceSha256,
    coreAssets: [...coreAssets].sort(compareAssetRecordNames),
  };
  return validateReleaseIntent(intent, { repository, version, sourceSha });
}

function validateReleaseIntent(intent, expected) {
  exactKeys(intent, ["schemaVersion", "repository", "version", "tag", "sourceSha", "signingRun", "buildInputSha256", "signingEvidenceSha256", "coreAssets"], "RELEASE-INTENT.json");
  assert(intent.schemaVersion === 1, "invalid-schema", "RELEASE-INTENT.json schemaVersion must be 1");
  assert(intent.repository === expected.repository && intent.version === expected.version && intent.sourceSha === expected.sourceSha, "invalid-schema", "RELEASE-INTENT.json source identity differs");
  assert(intent.tag === canonicalTag(expected.version), "invalid-schema", "RELEASE-INTENT.json tag differs");
  exactKeys(intent.signingRun, ["id", "attempt"], "RELEASE-INTENT.json signingRun");
  assertNonEmptyString(intent.signingRun.id, "RELEASE-INTENT.json signing run ID");
  assertNonEmptyString(intent.signingRun.attempt, "RELEASE-INTENT.json signing run attempt");
  assertHexSha256(intent.buildInputSha256, "RELEASE-INTENT.json buildInputSha256");
  assertHexSha256(intent.signingEvidenceSha256, "RELEASE-INTENT.json signingEvidenceSha256");
  assert(Array.isArray(intent.coreAssets), "invalid-schema", "RELEASE-INTENT.json coreAssets must be an array");
  const names = intent.coreAssets.map((asset) => asset.name).sort();
  assert(JSON.stringify(names) === JSON.stringify(coreAssetNames(expected.version)), "invalid-schema", "RELEASE-INTENT.json core asset set differs");
  for (const asset of intent.coreAssets) validateAssetRecord(asset, { ...expected, name: asset.name }, { requireId: false });
  return intent;
}

function validateBuildProvenance(provenance, expected, signingEvidence, buildInput) {
  exactKeys(provenance, ["schemaVersion", "version", "source", "release", "buildInput", "signing", "coreAssets"], "BUILD-PROVENANCE.json");
  assert(provenance.schemaVersion === 2 && provenance.version === expected.version, "invalid-schema", "BUILD-PROVENANCE.json version/schema differs");
  exactKeys(provenance.source, ["repository", "sha", "ref", "workflowRef", "runId", "runAttempt"], "BUILD-PROVENANCE.json source");
  assert(provenance.source.repository === expected.repository && provenance.source.sha === expected.sourceSha && provenance.source.ref === "refs/heads/main", "invalid-schema", "BUILD-PROVENANCE.json source differs");
  exactKeys(provenance.release, ["id", "tag", "draft", "canonicalBaseUrl"], "BUILD-PROVENANCE.json release");
  assert(Number(provenance.release.id) === Number(expected.releaseId) && provenance.release.tag === canonicalTag(expected.version) && provenance.release.draft === true, "invalid-schema", "BUILD-PROVENANCE.json release identity differs");
  assert(provenance.release.canonicalBaseUrl === `https://github.com/${expected.repository}/releases/download/${canonicalTag(expected.version)}`, "invalid-schema", "BUILD-PROVENANCE.json canonical base URL differs");
  validateBuildInput(provenance.buildInput, expected);
  validateSigningEvidence(provenance.signing, expected);
  assert(JSON.stringify(provenance.source) === JSON.stringify(signingEvidence.source), "invalid-schema", "BUILD-PROVENANCE.json source must equal signing evidence source");
  assert(
    provenance.buildInput.source.workflowRef === provenance.signing.source.workflowRef &&
      provenance.buildInput.source.runId === provenance.signing.source.runId &&
      provenance.buildInput.source.runAttempt === provenance.signing.source.runAttempt,
    "invalid-schema",
    "build and signing evidence must originate from the same workflow run",
  );
  assert(JSON.stringify(provenance.buildInput) === JSON.stringify(buildInput), "invalid-schema", "BUILD-PROVENANCE.json build input copy differs");
  assert(JSON.stringify(provenance.signing) === JSON.stringify(signingEvidence), "invalid-schema", "BUILD-PROVENANCE.json signing evidence copy differs");
  assert(Array.isArray(provenance.coreAssets), "invalid-schema", "BUILD-PROVENANCE.json coreAssets must be an array");
  const names = provenance.coreAssets.map((asset) => asset.name).sort();
  assert(JSON.stringify(names) === JSON.stringify(coreAssetNames(expected.version)), "invalid-schema", "BUILD-PROVENANCE.json core asset set differs");
  for (const asset of provenance.coreAssets) validateAssetRecord(asset, { ...expected, name: asset.name });
  return provenance;
}

function validateReleaseMetadata({ latest, provenance, signingEvidence, buildInput, assetRecords, signature, expected }) {
  assert(Array.isArray(assetRecords), "invalid-schema", "release asset records must be an array");
  const names = assetRecords.map((record) => record.name).sort();
  assert(JSON.stringify(names) === JSON.stringify(expectedAssetNames(expected.version)), "invalid-schema", "release asset record set differs");
  for (const record of assetRecords) validateAssetRecord(record, { ...expected, name: record.name });
  validateUpdaterManifest(latest, {
    repository: expected.repository,
    version: expected.version,
    assetName: `codex-manager-v${expected.version}-aarch64.app.tar.gz`,
    signature,
  });
  validateBuildInput(buildInput, expected);
  validateSigningEvidence(signingEvidence, expected);
  validateBuildProvenance(provenance, expected, signingEvidence, buildInput);
  const remoteByName = new Map(assetRecords.map((record) => [record.name, record]));
  for (const core of provenance.coreAssets) {
    assert(JSON.stringify(core) === JSON.stringify(remoteByName.get(core.name)), "invalid-schema", `BUILD-PROVENANCE.json asset record differs: ${core.name}`);
  }
  return true;
}

function validateAttestationIndex(index, version) {
  exactKeys(index, ["files"], "attestation index");
  assert(Array.isArray(index.files), "invalid-schema", "attestation index files must be an array");
  const expectedNames = expectedAssetNames(version).map((name) => `${name}.attestation.json`).sort();
  const actualNames = index.files.map((file) => file.name).sort();
  assert(JSON.stringify(actualNames) === JSON.stringify(expectedNames), "invalid-schema", "attestation evidence set differs");
  for (const file of index.files) {
    exactKeys(file, ["name", "sha256"], "attestation evidence file");
    assertHexSha256(file.sha256, `attestation evidence ${file.name}`);
  }
  return index;
}

function validateReleaseVerification(evidence, expected) {
  exactKeys(evidence, ["schemaVersion", "status", "verifiedAt", "source", "verificationRun", "release", "assets", "provenanceSha256", "attestations", "checks"], "RELEASE-VERIFICATION.json");
  assert(evidence.schemaVersion === 1, "invalid-schema", "RELEASE-VERIFICATION.json schemaVersion must be 1");
  const expectedStatus = expected.mode === "published" ? "published-publicly-verified" : "draft-remotely-verified";
  assert(evidence.status === expectedStatus, "invalid-schema", "RELEASE-VERIFICATION.json status differs");
  assert(RFC3339_UTC.test(evidence.verifiedAt) && !Number.isNaN(Date.parse(evidence.verifiedAt)), "invalid-schema", "RELEASE-VERIFICATION.json verifiedAt is invalid");
  exactKeys(evidence.source, ["repository", "sha", "ref"], "RELEASE-VERIFICATION.json source");
  assert(evidence.source.repository === expected.repository && evidence.source.sha === expected.sourceSha && evidence.source.ref === "refs/heads/main", "invalid-schema", "RELEASE-VERIFICATION.json source differs");
  exactKeys(evidence.verificationRun, ["workflowRef", "workflowSha", "runId", "runAttempt"], "RELEASE-VERIFICATION.json verificationRun");
  assertNonEmptyString(evidence.verificationRun.workflowRef, "RELEASE-VERIFICATION.json workflowRef");
  assert(SOURCE_SHA.test(evidence.verificationRun.workflowSha), "invalid-schema", "RELEASE-VERIFICATION.json workflow SHA is invalid");
  assert(/^[1-9][0-9]*$/.test(evidence.verificationRun.runId) && /^[1-9][0-9]*$/.test(evidence.verificationRun.runAttempt), "invalid-schema", "RELEASE-VERIFICATION.json run identity is invalid");
  exactKeys(evidence.release, ["id", "tag", "mode"], "RELEASE-VERIFICATION.json release");
  assert(Number(evidence.release.id) === Number(expected.releaseId) && evidence.release.tag === canonicalTag(expected.version) && evidence.release.mode === expected.mode, "invalid-schema", "RELEASE-VERIFICATION.json release identity differs");
  assert(Array.isArray(evidence.assets), "invalid-schema", "RELEASE-VERIFICATION.json assets must be an array");
  const names = evidence.assets.map((asset) => asset.name).sort();
  assert(JSON.stringify(names) === JSON.stringify(expectedAssetNames(expected.version)), "invalid-schema", "RELEASE-VERIFICATION.json asset set differs");
  for (const asset of evidence.assets) validateAssetRecord(asset, { ...expected, name: asset.name });
  assertHexSha256(evidence.provenanceSha256, "RELEASE-VERIFICATION.json provenanceSha256");
  const provenanceAsset = evidence.assets.find((asset) => asset.name === "BUILD-PROVENANCE.json");
  assert(
    provenanceAsset?.digest === `sha256:${evidence.provenanceSha256}`,
    "invalid-schema",
    "RELEASE-VERIFICATION.json provenanceSha256 differs from the verified asset digest",
  );
  validateAttestationIndex(evidence.attestations, expected.version);
  exactKeys(evidence.checks, ["exactAssetSetAndDigests", "updaterSignatureAndStableUrl", "appAndDmgTrustChain", "sbomAndLicenses", "anonymousStableDownloads", "latestAlias"], "RELEASE-VERIFICATION.json checks");
  for (const name of ["exactAssetSetAndDigests", "updaterSignatureAndStableUrl", "appAndDmgTrustChain", "sbomAndLicenses"]) {
    assert(evidence.checks[name] === true, "invalid-schema", `RELEASE-VERIFICATION.json check is not true: ${name}`);
  }
  const published = expected.mode === "published";
  assert(evidence.checks.anonymousStableDownloads === published && evidence.checks.latestAlias === published, "invalid-schema", "RELEASE-VERIFICATION.json public download checks differ");
  return evidence;
}

function validateUpdaterManifest(manifest, expected) {
  exactKeys(manifest, ["version", "notes", "pub_date", "platforms"], "latest.json");
  assert(manifest.version === expected.version, "invalid-updater-manifest", "latest.json version differs");
  assert(RFC3339_UTC.test(manifest.pub_date) && !Number.isNaN(Date.parse(manifest.pub_date)), "invalid-updater-manifest", "latest.json pub_date is not RFC3339 UTC");
  exactKeys(manifest.platforms, ["darwin-aarch64", "darwin-aarch64-app"], "latest.json platforms");
  const stableUrl = canonicalAssetUrl(expected.repository, expected.version, expected.assetName);
  for (const platform of Object.values(manifest.platforms)) {
    exactKeys(platform, ["signature", "url"], "latest.json platform");
    assert(platform.signature === expected.signature, "invalid-updater-manifest", "latest.json signature differs");
    assert(platform.url === stableUrl, "invalid-updater-manifest", "latest.json must use the canonical version-tag URL");
    assert(!platform.url.includes("/untagged-"), "invalid-updater-manifest", "latest.json must never use an untagged draft URL");
  }
  return manifest;
}

function validatePublishedReleaseState({ release, latestRelease, tagSha, expected }) {
  validateReleaseIdentity(release, expected, { published: true });
  assert(Number(latestRelease?.id) === Number(expected.releaseId), "latest-release-mismatch", "releases/latest does not resolve to the expected release");
  assert(tagSha === expected.sourceSha, "tag-source-mismatch", "release tag does not resolve to the expected source SHA");
  return true;
}

function classifyPublishedReleaseState({ release, latestRelease, tagSha }, expected) {
  assert(release && typeof release === "object", "invalid-release", "release response must be an object");
  assert(Number(release.id) === Number(expected.releaseId), "release-identity-mismatch", "release ID does not match");
  assert(release.tag_name === canonicalTag(expected.version), "release-identity-mismatch", "release tag does not match");
  assert(release.target_commitish === expected.sourceSha, "release-identity-mismatch", "release source SHA does not match");
  assert(release.prerelease === false, "release-identity-mismatch", "release must not be a prerelease");
  assert(typeof release.draft === "boolean", "release-identity-mismatch", "release draft state is invalid");
  assert(tagSha === expected.sourceSha, "tag-source-mismatch", "release tag does not resolve to the expected source SHA");
  if (release.draft || typeof release.published_at !== "string" || release.published_at.length === 0) {
    return { kind: "retry", reason: "published Release transition is not visible" };
  }
  if (Number(latestRelease?.id) !== Number(expected.releaseId)) {
    return { kind: "retry", reason: "releases/latest has not transitioned to the expected Release" };
  }
  return { kind: "ready", value: release };
}

async function awaitPublishedReleaseState(readState, expected, options = {}) {
  return retryUntil(readState, (state) => classifyPublishedReleaseState(state, expected), {
    ...options,
    label: options.label ?? "published Release transition",
  });
}

async function dryRun() {
  let milliseconds = 0;
  const clock = { now: () => milliseconds, sleep: async (delay) => { milliseconds += delay; } };
  let calls = 0;
  const release = { id: 42, tag_name: "v0.2.1", target_commitish: "a".repeat(40), draft: true, prerelease: false };
  await awaitVisibleRelease(
    async () => (++calls < 4 ? [] : [release]),
    { releaseId: 42, version: "0.2.1", sourceSha: "a".repeat(40) },
    { clock, deadlineMs: 30_000 },
  );
  const manifest = buildStableUpdaterManifest({
    repository: "owner/repository",
    version: "0.2.1",
    assetName: "codex-manager-v0.2.1-aarch64.app.tar.gz",
    signature: "synthetic-signature-material",
    publishedAt: "2026-08-29T00:00:00Z",
    notes: "synthetic dry run",
  });
  validateUpdaterManifest(manifest, {
    repository: "owner/repository",
    version: "0.2.1",
    assetName: "codex-manager-v0.2.1-aarch64.app.tar.gz",
    signature: "synthetic-signature-material",
  });
  return { releaseVisibilityCalls: calls, stableUrl: manifest.platforms["darwin-aarch64"].url };
}

export {
  API_DIGEST,
  DEFAULT_DEADLINE_MS,
  HEX_SHA256,
  HttpStatusError,
  ReleaseProtocolError,
  SOURCE_SHA,
  awaitExpectedAssets,
  awaitPublishedReleaseState,
  awaitVisibleRelease,
  buildStableUpdaterManifest,
  buildReleaseIntent,
  canonicalAssetUrl,
  canonicalTag,
  classifyAssetSet,
  classifyPublishedReleaseState,
  classifyReleaseVisibility,
  compareAssetRecordNames,
  coreAssetNames,
  downloadWithRetry,
  dryRun,
  expectedAssetNames,
  fileRecord,
  isTransientError,
  retryUntil,
  sha256Bytes,
  uploadOrReconcile,
  validatePublishedReleaseState,
  validateBuildInput,
  validateBuildProvenance,
  validateReleaseIntent,
  validateReleaseMetadata,
  validateReleaseVerification,
  validateReleaseIdentity,
  validateSigningEvidence,
  validateUpdaterManifest,
  verifyAttestationWithRetry,
};

async function main() {
  if (process.argv[2] !== "dry-run" || process.argv.length !== 3) {
    throw new ReleaseProtocolError("invalid-command", "usage: release-state.mjs dry-run");
  }
  const result = await dryRun();
  process.stdout.write(`${JSON.stringify({ status: "ok", ...result })}\n`);
}

if (resolve(process.argv[1] ?? "") === fileURLToPath(import.meta.url)) {
  main().catch((error) => {
    process.stderr.write(`release control failed: ${error.message}\n`);
    process.exitCode = 1;
  });
}
