import assert from "node:assert/strict";
import { test } from "node:test";
import { AuditPolicyError, verifyAuditReport } from "../scripts/verify-cargo-audit.mjs";
import { verifyNpmAuditReport } from "../scripts/verify-npm-audit.mjs";

test("npm audit policy 只接受所有等级均为零的完整报告", () => {
  const clean = {
    auditReportVersion: 2,
    vulnerabilities: {},
    metadata: { vulnerabilities: { info: 0, low: 0, moderate: 0, high: 0, critical: 0, total: 0 } },
  };
  assert.deepEqual(verifyNpmAuditReport(clean), { vulnerabilities: 0 });
  assert.throws(
    () => verifyNpmAuditReport({
      ...clean,
      vulnerabilities: { synthetic: { severity: "moderate" } },
      metadata: { vulnerabilities: { ...clean.metadata.vulnerabilities, moderate: 1, total: 1 } },
    }),
    /one or more vulnerabilities/,
  );
  assert.throws(
    () => verifyNpmAuditReport({ ...clean, metadata: { vulnerabilities: { ...clean.metadata.vulnerabilities, total: 1 } } }),
    /inconsistent/,
  );
  assert.throws(
    () => verifyNpmAuditReport({
      ...clean,
      metadata: { vulnerabilities: { ...clean.metadata.vulnerabilities, synthetic: 0 } },
    }),
    /count fields are invalid/,
  );
});

const allowlist = {
  schemaVersion: 1,
  reviewedAt: "2026-08-29",
  releaseTarget: "aarch64-apple-darwin",
  warnings: [
    { kind: "unsound", id: "RUSTSEC-2024-0429", package: "glib", version: "0.18.5", targetExcluded: true, rationale: "Synthetic target exclusion rationale long enough for policy review." },
  ],
};
const report = {
  vulnerabilities: { count: 0 },
  warnings: {
    unsound: [{ advisory: { id: "RUSTSEC-2024-0429" }, package: { name: "glib", version: "0.18.5" } }],
  },
};

test("cargo audit policy 接受已审查且不进入发布目标图的 warning", () => {
  assert.deepEqual(verifyAuditReport(report, allowlist, "codex-manager v0.2.1\n"), { vulnerabilities: 0, reviewedWarnings: 1 });
});

test("cargo audit policy 拒绝新增 warning、漏洞和 target exclusion 失效", () => {
  const added = structuredClone(report);
  (added.warnings as Record<string, unknown>).yanked = [{ advisory: { id: "RUSTSEC-2099-0001" }, package: { name: "synthetic", version: "1.0.0" } }];
  assert.throws(() => verifyAuditReport(added, allowlist, ""), AuditPolicyError);
  const vulnerable = structuredClone(report);
  vulnerable.vulnerabilities.count = 1;
  assert.throws(() => verifyAuditReport(vulnerable, allowlist, ""), AuditPolicyError);
  assert.throws(() => verifyAuditReport(report, allowlist, "glib v0.18.5\n"), AuditPolicyError);
  assert.throws(
    () => verifyAuditReport({ vulnerabilities: { count: 0 }, warnings: {} }, allowlist, ""),
    /allowlist entry is no longer present/,
  );
});
