#!/usr/bin/env node

import { mkdir, readFile, readdir, rename, rm, writeFile } from "node:fs/promises";
import { basename, dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import {
  ReleaseProtocolError,
  buildReleaseIntent,
  canonicalTag,
  compareAssetRecordNames,
  coreAssetNames,
  fileRecord,
  sha256Bytes,
  validateBuildInput,
  validateBuildProvenance,
  validateReleaseIntent,
  validateReleaseMetadata,
  validateReleaseVerification,
  validateSigningEvidence,
} from "./release-state.mjs";

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
    if (!key.startsWith("--") || index + 1 >= argv.length) throw new ReleaseProtocolError("invalid-command", `invalid command argument: ${key ?? "<missing>"}`);
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

async function readJson(path) {
  return JSON.parse(await readFile(path, "utf8"));
}

async function atomicJson(path, value) {
  await mkdir(dirname(path), { recursive: true });
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

async function coreRecords(assetDirectory, repository, version) {
  const expected = coreAssetNames(version);
  const entries = await readdir(assetDirectory, { withFileTypes: true });
  if (entries.some((entry) => !entry.isFile())) throw new ReleaseProtocolError("core-asset-set-mismatch", "signed core asset directory contains a non-file entry");
  const actual = entries.map((entry) => entry.name).sort();
  if (JSON.stringify(actual) !== JSON.stringify(expected)) throw new ReleaseProtocolError("core-asset-set-mismatch", "signed core asset set is incomplete");
  return Promise.all(actual.map((name) => fileRecord(join(assetDirectory, name), repository, version)));
}

function canonicalAssetRecords(records) {
  return [...records].sort(compareAssetRecordNames);
}

async function makeIntent(args, context) {
  const buildInputBytes = await readFile(args.get("build-input"));
  const signingEvidenceBytes = await readFile(args.get("signing-evidence"));
  const buildInput = JSON.parse(buildInputBytes.toString("utf8"));
  const signingEvidence = JSON.parse(signingEvidenceBytes.toString("utf8"));
  validateBuildInput(buildInput, context);
  validateSigningEvidence(signingEvidence, context);
  const intent = buildReleaseIntent({
    ...context,
    signingRunId: requiredEnvironment("GITHUB_RUN_ID"),
    signingRunAttempt: requiredEnvironment("GITHUB_RUN_ATTEMPT"),
    buildInputSha256: sha256Bytes(buildInputBytes),
    signingEvidenceSha256: sha256Bytes(signingEvidenceBytes),
    coreAssets: await coreRecords(args.get("asset-dir"), context.repository, context.version),
  });
  await atomicJson(args.get("output"), intent);
}

async function validateIntentFiles(args, context) {
  const intent = await readJson(args.get("intent"));
  const buildInputBytes = await readFile(args.get("build-input"));
  const signingEvidenceBytes = await readFile(args.get("signing-evidence"));
  const signingEvidence = JSON.parse(signingEvidenceBytes.toString("utf8"));
  validateReleaseIntent(intent, context);
  validateBuildInput(JSON.parse(buildInputBytes.toString("utf8")), context);
  validateSigningEvidence(signingEvidence, context);
  if (intent.buildInputSha256 !== sha256Bytes(buildInputBytes) || intent.signingEvidenceSha256 !== sha256Bytes(signingEvidenceBytes)) {
    throw new ReleaseProtocolError("intent-hash-mismatch", "release intent does not bind the supplied build/signing evidence");
  }
  if (intent.signingRun.id !== signingEvidence.source.runId || intent.signingRun.attempt !== signingEvidence.source.runAttempt) {
    throw new ReleaseProtocolError("intent-run-mismatch", "release intent does not bind the signing workflow run");
  }
  const local = await coreRecords(args.get("asset-dir"), context.repository, context.version);
  if (JSON.stringify(canonicalAssetRecords(intent.coreAssets)) !== JSON.stringify(canonicalAssetRecords(local))) {
    throw new ReleaseProtocolError("intent-asset-mismatch", "release intent does not bind the supplied core assets");
  }
}

async function makeProvenance(args, context) {
  const buildInput = await readJson(args.get("build-input"));
  const signingEvidence = await readJson(args.get("signing-evidence"));
  const records = await readJson(args.get("core-records"));
  const releaseId = Number(args.get("release-id"));
  const provenance = {
    schemaVersion: 2,
    version: context.version,
    source: {
      ...signingEvidence.source,
    },
    release: {
      id: releaseId,
      tag: canonicalTag(context.version),
      draft: true,
      canonicalBaseUrl: `https://github.com/${context.repository}/releases/download/${canonicalTag(context.version)}`,
    },
    buildInput,
    signing: signingEvidence,
    coreAssets: records,
  };
  validateBuildProvenance(provenance, { ...context, releaseId }, signingEvidence, buildInput);
  await atomicJson(args.get("output"), provenance);
}

async function validateMetadataFiles(args, context) {
  const releaseId = Number(args.get("release-id"));
  const assetDirectory = args.get("asset-dir");
  const signature = (await readFile(join(assetDirectory, `codex-manager-v${context.version}-aarch64.app.tar.gz.sig`), "utf8")).replace(/[\r\n]/g, "");
  validateReleaseMetadata({
    latest: await readJson(join(assetDirectory, "latest.json")),
    provenance: await readJson(join(assetDirectory, "BUILD-PROVENANCE.json")),
    signingEvidence: await readJson(join(assetDirectory, "SIGNING-EVIDENCE.json")),
    buildInput: await readJson(args.get("build-input")),
    assetRecords: await readJson(args.get("asset-records")),
    signature,
    expected: { ...context, releaseId },
  });
}

async function makeVerification(args, context) {
  const mode = args.get("mode");
  if (!new Set(["draft", "published"]).has(mode)) throw new ReleaseProtocolError("invalid-command", "verification mode must be draft or published");
  const records = await readJson(args.get("asset-records"));
  const attestations = await readJson(args.get("attestations-index"));
  const provenanceBytes = await readFile(args.get("provenance"));
  const releaseId = Number(args.get("release-id"));
  const evidence = {
    schemaVersion: 1,
    status: mode === "published" ? "published-publicly-verified" : "draft-remotely-verified",
    verifiedAt: new Date().toISOString().replace(/\.\d{3}Z$/, "Z"),
    source: { repository: context.repository, sha: context.sourceSha, ref: "refs/heads/main" },
    verificationRun: {
      workflowRef: requiredEnvironment("GITHUB_WORKFLOW_REF"),
      workflowSha: requiredEnvironment("GITHUB_WORKFLOW_SHA"),
      runId: requiredEnvironment("GITHUB_RUN_ID"),
      runAttempt: requiredEnvironment("GITHUB_RUN_ATTEMPT"),
    },
    release: { id: releaseId, tag: canonicalTag(context.version), mode },
    assets: records,
    provenanceSha256: sha256Bytes(provenanceBytes),
    attestations,
    checks: {
      exactAssetSetAndDigests: true,
      updaterSignatureAndStableUrl: true,
      appAndDmgTrustChain: true,
      sbomAndLicenses: true,
      anonymousStableDownloads: mode === "published",
      latestAlias: mode === "published",
    },
  };
  validateReleaseVerification(evidence, { ...context, releaseId, mode });
  await atomicJson(args.get("output"), evidence);
}

async function main() {
  const args = parseArgs(process.argv.slice(2));
  const context = {
    repository: requiredEnvironment("REPOSITORY"),
    version: requiredEnvironment("VERSION"),
    sourceSha: requiredEnvironment("SOURCE_SHA"),
  };
  if (args.command === "make-intent") return makeIntent(args, context);
  if (args.command === "validate-intent") return validateIntentFiles(args, context);
  if (args.command === "make-provenance") return makeProvenance(args, context);
  if (args.command === "validate-metadata") return validateMetadataFiles(args, context);
  if (args.command === "make-verification") return makeVerification(args, context);
  throw new ReleaseProtocolError("invalid-command", `unsupported metadata command: ${args.command ?? "<missing>"}`);
}

export { coreRecords, makeIntent, makeProvenance, makeVerification, validateIntentFiles, validateMetadataFiles };

if (resolve(process.argv[1] ?? "") === fileURLToPath(import.meta.url)) {
  main().catch((error) => {
    process.stderr.write(`release metadata control failed: ${error.message}\n`);
    process.exitCode = 1;
  });
}
