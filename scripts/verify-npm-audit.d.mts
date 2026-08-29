export class NpmAuditPolicyError extends Error {}
export function verifyNpmAuditReport(report: any): { vulnerabilities: number };
