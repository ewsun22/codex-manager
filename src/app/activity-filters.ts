import type { ActivityQuery } from "../shared/contracts.ts";

export function facetOptions(values: string[], selected?: string | null): string[] {
  return [...new Set([...values, ...(selected ? [selected] : [])])].sort();
}

export function queryForView(query: ActivityQuery, view: NonNullable<ActivityQuery["view"]>): ActivityQuery {
  return { ...query, view, cursor: null, result: view === "turns" && query.result === "accounted" ? null : query.result };
}
