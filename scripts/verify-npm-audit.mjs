#!/usr/bin/env node

import { readFile } from "node:fs/promises";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";

class NpmAuditPolicyError extends Error {}

function assert(condition, message) {
  if (!condition) throw new NpmAuditPolicyError(message);
}

function verifyNpmAuditReport(report) {
  assert(report && typeof report === "object", "npm audit report is invalid");
  assert(report.auditReportVersion === 2, "npm audit report version is unsupported");
  assert(report.vulnerabilities && typeof report.vulnerabilities === "object" && !Array.isArray(report.vulnerabilities), "npm audit vulnerabilities are invalid");
  const counts = report.metadata?.vulnerabilities;
  assert(counts && typeof counts === "object" && !Array.isArray(counts), "npm audit vulnerability counts are missing");
  const severities = ["info", "low", "moderate", "high", "critical"];
  assert(
    JSON.stringify(Object.keys(counts).sort()) === JSON.stringify([...severities, "total"].sort()),
    "npm audit vulnerability count fields are invalid",
  );
  for (const severity of severities) {
    assert(Number.isSafeInteger(counts[severity]) && counts[severity] >= 0, `npm audit ${severity} count is invalid`);
  }
  assert(Number.isSafeInteger(counts.total) && counts.total >= 0, "npm audit total count is invalid");
  assert(counts.total === severities.reduce((sum, severity) => sum + counts[severity], 0), "npm audit vulnerability counts are inconsistent");
  assert(counts.total === 0 && Object.keys(report.vulnerabilities).length === 0, "npm audit reported one or more vulnerabilities");
  return { vulnerabilities: 0 };
}

async function main() {
  if (process.argv.length !== 3) throw new NpmAuditPolicyError("usage: verify-npm-audit.mjs REPORT.json");
  const report = JSON.parse(await readFile(process.argv[2], "utf8"));
  const result = verifyNpmAuditReport(report);
  process.stdout.write(`npm_audit_policy=ok vulnerabilities=${result.vulnerabilities}\n`);
}

export { NpmAuditPolicyError, verifyNpmAuditReport };

if (resolve(process.argv[1] ?? "") === fileURLToPath(import.meta.url)) {
  main().catch((error) => {
    process.stderr.write(`npm audit policy failed: ${error.message}\n`);
    process.exitCode = 1;
  });
}
