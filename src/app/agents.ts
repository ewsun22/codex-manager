import type { AgentsChain, AgentsFileSummary } from "../shared/contracts.ts";

function basename(path: string): string {
  return path.replace(/\\/g, "/").split("/").at(-1) ?? "";
}

/**
 * Select the first file users should inspect for a project. Global AGENTS.md
 * is deliberately preferred even when it is read-only or overridden: it is
 * the top-level rule that applies before project-specific files.
 */
export function preferredAgentsFile(chain: AgentsChain): AgentsFileSummary | null {
  return chain.files.find((file) => file.kind === "global" && basename(file.path) === "AGENTS.md")
    ?? chain.files.find((file) => file.kind === "global")
    ?? chain.files.find((file) => file.kind === "project")
    ?? chain.files[0]
    ?? null;
}
