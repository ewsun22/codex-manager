#!/usr/bin/env node

import { readFile } from "node:fs/promises";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";

class AuditPolicyError extends Error {}

function key(entry) {
  return [entry.kind, entry.id, entry.package, entry.version].join("|");
}

function assert(condition, message) {
  if (!condition) throw new AuditPolicyError(message);
}

function verifyAuditReport(report, allowlist, targetTree) {
  assert(report && typeof report === "object", "cargo-audit report is invalid");
  assert(Number(report.vulnerabilities?.count) === 0, "cargo-audit reported one or more vulnerabilities");
  assert(allowlist?.schemaVersion === 1 && allowlist.releaseTarget === "aarch64-apple-darwin", "cargo-audit allowlist schema is invalid");
  assert(JSON.stringify(Object.keys(allowlist).sort()) === JSON.stringify(["releaseTarget", "reviewedAt", "schemaVersion", "warnings"]), "cargo-audit allowlist fields are invalid");
  assert(/^\d{4}-\d{2}-\d{2}$/.test(allowlist.reviewedAt), "cargo-audit allowlist review date is invalid");
  assert(Array.isArray(allowlist.warnings), "cargo-audit allowlist warnings are invalid");

  const allowed = new Map();
  for (const entry of allowlist.warnings) {
    assert(JSON.stringify(Object.keys(entry).sort()) === JSON.stringify(["id", "kind", "package", "rationale", "targetExcluded", "version"]), `cargo-audit allowlist fields differ for ${entry.id}`);
    assert(["unmaintained", "unsound"].includes(entry.kind), `unsupported allowlist warning kind: ${entry.kind}`);
    assert(/^RUSTSEC-\d{4}-\d{4}$/.test(entry.id), `invalid advisory ID in allowlist: ${entry.id}`);
    assert(typeof entry.package === "string" && entry.package.length > 0, `missing package for ${entry.id}`);
    assert(typeof entry.version === "string" && entry.version.length > 0, `missing version for ${entry.id}`);
    assert(typeof entry.targetExcluded === "boolean", `missing target exclusion decision for ${entry.id}`);
    assert(typeof entry.rationale === "string" && entry.rationale.length >= 40, `missing review rationale for ${entry.id}`);
    const entryKey = key(entry);
    assert(!allowed.has(entryKey), `duplicate cargo-audit allowlist entry: ${entryKey}`);
    allowed.set(entryKey, entry);
    if (entry.targetExcluded) {
      const packageLine = `${entry.package} v${entry.version}`;
      assert(!targetTree.split("\n").includes(packageLine), `target-excluded advisory entered the macOS release graph: ${entry.id}`);
    }
  }

  const actual = [];
  for (const [kind, values] of Object.entries(report.warnings ?? {})) {
    assert(Array.isArray(values), `cargo-audit warning category is invalid: ${kind}`);
    for (const value of values) {
      actual.push({ kind, id: value.advisory?.id, package: value.package?.name, version: value.package?.version });
    }
  }
  for (const warning of actual) {
    assert(allowed.has(key(warning)), `unreviewed cargo-audit warning: ${key(warning)}`);
  }
  const actualKeys = new Set(actual.map(key));
  for (const allowedKey of allowed.keys()) {
    assert(actualKeys.has(allowedKey), `cargo-audit allowlist entry is no longer present: ${allowedKey}`);
  }
  assert(actualKeys.size === actual.length, "cargo-audit report contains duplicate warning identities");
  return { vulnerabilities: 0, reviewedWarnings: actual.length };
}

function parseArgs(argv) {
  const values = new Map();
  for (let index = 0; index < argv.length; index += 2) {
    assert(argv[index]?.startsWith("--") && argv[index + 1], "usage: verify-cargo-audit.mjs --audit FILE --allowlist FILE --target-tree FILE");
    values.set(argv[index].slice(2), argv[index + 1]);
  }
  for (const name of ["audit", "allowlist", "target-tree"]) assert(values.has(name), `missing --${name}`);
  return values;
}

async function main() {
  const args = parseArgs(process.argv.slice(2));
  const [report, allowlist, targetTree] = await Promise.all([
    readFile(args.get("audit"), "utf8").then(JSON.parse),
    readFile(args.get("allowlist"), "utf8").then(JSON.parse),
    readFile(args.get("target-tree"), "utf8"),
  ]);
  const result = verifyAuditReport(report, allowlist, targetTree);
  process.stdout.write(`cargo_audit_policy=ok vulnerabilities=${result.vulnerabilities} reviewed_warnings=${result.reviewedWarnings}\n`);
}

export { AuditPolicyError, verifyAuditReport };

if (resolve(process.argv[1] ?? "") === fileURLToPath(import.meta.url)) {
  main().catch((error) => {
    process.stderr.write(`cargo audit policy failed: ${error.message}\n`);
    process.exitCode = 1;
  });
}
