export class AuditPolicyError extends Error {}
export function verifyAuditReport(
  report: any,
  allowlist: any,
  targetTree: string,
): { vulnerabilities: number; reviewedWarnings: number };
